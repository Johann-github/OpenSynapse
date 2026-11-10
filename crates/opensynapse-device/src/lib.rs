//! Device reads and safe writes, independent of the UI and of the real backend.
//!
//! Shared by the desktop GUI and the background agent. See README.md.
//!
//! Everything here is generic over [`FeatureIo`], so it runs against the real
//! device and against a fake mouse (`fake` module) in the tests. Report bytes
//! come only from `opensynapse-protocol` builders.
//!
//! Write safety mirrors the CLI: check the value against the registry, refuse while
//! Razer software runs unless the user confirmed, read the old value, write, read
//! back, and hand the old value to the UI so it can offer "Restore".

use std::fmt;
use std::ops::RangeInclusive;

use opensynapse_device_registry::Device;
use opensynapse_hid::{FeatureIo, HidError, Session};
use opensynapse_protocol::buttons::{self, ButtonAssignment, Known, Mode};
use opensynapse_protocol::device::{self, DeviceMode, FirmwareVersion};
use opensynapse_protocol::power::{self, BatteryLevel, ChargingStatus};
use opensynapse_protocol::{ProtocolError, Request, Status, dpi, hid_usage, polling};

/// Profile byte for the HyperPolling variant (same default as the CLI; meaning
/// unconfirmed).
pub const HYPER_PROFILE: u8 = 0;
/// Onboard profile whose button assignments are shown and edited (CLI default).
pub const BUTTON_PROFILE: u8 = 1;
/// Polling rates offered when the registry lists none.
pub const FALLBACK_POLLING_RATES: [u32; 3] = [125, 500, 1000];
/// Mouse buttons a button may be mapped to (1 left, 2 right, 3 middle, 4/5 side).
pub const MOUSE_TARGETS: RangeInclusive<u8> = 1..=5;
/// Left and right click: losing them makes the mouse hard to use, so writes to
/// them need [`WriteOpts::allow_primary`].
pub const PRIMARY_BUTTONS: [u8; 2] = [1, 2];
/// Stages the editor offers at most when the device reports fewer. The registry
/// has no stage count; the Viper V3 HyperSpeed ships 5 stages (observed, vault
/// `Devices/Viper V3 HyperSpeed`), so the GUI does not go beyond what was seen.
pub const DEFAULT_MAX_STAGES: usize = 5;

/// A value read from the device, or why it could not be read.
pub type Field<T> = Result<T, String>;

/// Display name of a button ID (Viper V3 HyperSpeed layout, 02/8E list).
pub fn button_name(id: u8) -> String {
    match id {
        1 => "Left click".into(),
        2 => "Right click".into(),
        3 => "Wheel click".into(),
        4 => "Side button 4".into(),
        5 => "Side button 5".into(),
        9 => "Wheel up".into(),
        10 => "Wheel down".into(),
        96 => "DPI button".into(),
        n => format!("Button {n}"),
    }
}

/// Which polling-rate command pair the device answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollingVariant {
    /// 00/05 + 00/85.
    Period,
    /// 00/40 + 00/C0.
    Hyper,
}

impl fmt::Display for PollingVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Period => "period",
            Self::Hyper => "hyper",
        })
    }
}

/// One writable setting with its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    /// Active DPI, volatile storage (the CLI default; lost on power-off).
    Dpi(dpi::Dpi),
    Polling {
        hz: u32,
        variant: PollingVariant,
    },
    /// Sleep timer in seconds.
    Sleep(u16),
    /// One button slot (profile, button, mode) with its action.
    Button(ButtonAssignment),
    /// The DPI stage list and the active stage. Setting the active stage also
    /// applies its DPI (observed).
    DpiStages {
        storage: dpi::Storage,
        stages: dpi::DpiStages,
    },
}

/// `mouse button 4`, or the raw function and data when the layout is unknown.
pub fn action_text(a: &ButtonAssignment) -> String {
    match a.known() {
        Some(Known::Key { modifiers, usage }) => key_text(modifiers, usage),
        Some(k) => k.to_string(),
        None => format!("other (function {}, data {:?})", a.function, a.data),
    }
}

/// `ctrl+shift+a`, falling back to hex for keys without a name.
fn key_text(modifiers: u8, usage: u8) -> String {
    let key = hid_usage::name(usage).unwrap_or_else(|| format!("{usage:#04x}"));
    match hid_usage::modifier_names(modifiers) {
        Some(mods) if !mods.is_empty() => format!("key {}+{key}", mods.join("+")),
        Some(_) => format!("key {key}"),
        None => format!("key {key} (mods {modifiers:#04x})"),
    }
}

fn storage_text(s: dpi::Storage) -> &'static str {
    match s {
        dpi::Storage::Volatile => "volatile",
        dpi::Storage::Persistent => "persistent",
    }
}

/// `400 [800] 1600`: the active stage in brackets.
pub fn stages_text(s: &dpi::DpiStages) -> String {
    s.stages
        .iter()
        .map(|st| {
            if st.id == s.active {
                format!("[{}]", st.dpi)
            } else {
                st.dpi.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dpi(d) => write!(f, "DPI {d}"),
            Self::Polling { hz, .. } => write!(f, "polling rate {hz} Hz"),
            Self::Sleep(v) => write!(f, "sleep timer {v}"),
            Self::Button(a) => write!(
                f,
                "{} ({}): {}",
                button_name(a.button),
                a.mode,
                action_text(a)
            ),
            Self::DpiStages { storage, stages } => write!(
                f,
                "DPI stages {} ({})",
                stages_text(stages),
                storage_text(*storage)
            ),
        }
    }
}

impl Setting {
    fn write_request(&self) -> Result<Request, ProtocolError> {
        match self {
            Self::Dpi(d) => Ok(dpi::set_dpi(dpi::Storage::Volatile, *d)),
            Self::Polling {
                hz,
                variant: PollingVariant::Period,
            } => polling::set_polling_rate(*hz),
            Self::Polling {
                hz,
                variant: PollingVariant::Hyper,
            } => polling::set_hyper_polling(HYPER_PROFILE, *hz),
            Self::Sleep(v) => Ok(power::set_sleep_time(*v)),
            Self::Button(a) => buttons::set_button_assignment(a),
            Self::DpiStages { storage, stages } => dpi::set_dpi_stages(*storage, stages),
        }
    }

    /// Read the current value of the same setting (same polling variant, same
    /// button slot, same stage storage).
    fn read_current<D: FeatureIo>(&self, s: &mut Session<D>) -> Result<Setting, HidError> {
        Ok(match self {
            Self::Dpi(_) => Self::Dpi(query(
                s,
                &dpi::get_dpi(dpi::Storage::Volatile),
                dpi::parse_dpi,
            )?),
            Self::Polling { variant, .. } => Self::Polling {
                hz: read_polling_variant(s, *variant)?,
                variant: *variant,
            },
            Self::Sleep(_) => {
                Self::Sleep(query(s, &power::get_sleep_time(), power::parse_sleep_time)?)
            }
            Self::Button(a) => {
                let req = buttons::get_button_assignment(a.profile, a.button, a.mode);
                let mode = a.mode;
                Self::Button(
                    query(s, &req, |x| buttons::parse_button_assignment(x, mode))?.assignment,
                )
            }
            Self::DpiStages { storage, .. } => Self::DpiStages {
                storage: *storage,
                stages: query(s, &dpi::get_dpi_stages(*storage), dpi::parse_dpi_stages)?,
            },
        })
    }
}

fn query<D: FeatureIo, T>(
    s: &mut Session<D>,
    req: &Request,
    parse: impl FnOnce(&[u8]) -> Result<T, ProtocolError>,
) -> Result<T, HidError> {
    let reply = s.execute(req)?;
    Ok(parse(reply.args())?)
}

/// Keep a per-field error as text for the panel.
fn f<T>(r: Result<T, HidError>) -> Field<T> {
    r.map_err(|e| e.to_string())
}

fn read_polling_variant<D: FeatureIo>(
    s: &mut Session<D>,
    variant: PollingVariant,
) -> Result<u32, HidError> {
    match variant {
        PollingVariant::Period => {
            query(s, &polling::get_polling_rate(), polling::parse_polling_rate)
        }
        PollingVariant::Hyper => query(
            s,
            &polling::get_hyper_polling(HYPER_PROFILE),
            polling::parse_hyper_polling,
        ),
    }
}

/// Polling rate via the period variant, falling back to HyperPolling when the
/// device rejects the period command (same rule as the CLI).
pub fn read_polling<D: FeatureIo>(s: &mut Session<D>) -> Result<(u32, PollingVariant), HidError> {
    match read_polling_variant(s, PollingVariant::Period) {
        Ok(hz) => Ok((hz, PollingVariant::Period)),
        Err(HidError::Status(
            Status::NotSupported | Status::UnknownCommand | Status::WrongDeviceMode,
        )) => Ok((
            read_polling_variant(s, PollingVariant::Hyper)?,
            PollingVariant::Hyper,
        )),
        Err(e) => Err(e),
    }
}

/// All button slots of one mode via 02/8E (first page, ID offset 0; the Viper's 8
/// buttons fit in it). The result's `mode` is the requested one.
pub fn read_button_list<D: FeatureIo>(
    s: &mut Session<D>,
    mode: Mode,
) -> Result<Vec<ButtonAssignment>, HidError> {
    let req = buttons::get_button_assignment_list(BUTTON_PROFILE, mode, 0);
    let list = query(s, &req, buttons::parse_button_assignment_list)?;
    Ok(list
        .records()
        .into_iter()
        .map(|(button, function, data)| ButtonAssignment {
            profile: BUTTON_PROFILE,
            button,
            mode,
            function,
            data,
        })
        .collect())
}

/// One row of the button table: both slots of a button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonRow {
    pub button: u8,
    pub normal: Option<ButtonAssignment>,
    pub hypershift: Option<ButtonAssignment>,
}

impl ButtonRow {
    pub fn slot(&self, mode: Mode) -> Option<&ButtonAssignment> {
        match mode {
            Mode::Normal => self.normal.as_ref(),
            Mode::HyperShift => self.hypershift.as_ref(),
        }
    }
}

/// Merge the two lists by button ID, in the device's normal-mode order; buttons
/// only in the HyperShift list come last.
pub fn button_rows(normal: &[ButtonAssignment], hypershift: &[ButtonAssignment]) -> Vec<ButtonRow> {
    let mut ids: Vec<u8> = normal.iter().map(|a| a.button).collect();
    for a in hypershift {
        if !ids.contains(&a.button) {
            ids.push(a.button);
        }
    }
    let find = |list: &[ButtonAssignment], b: u8| list.iter().find(|a| a.button == b).cloned();
    ids.into_iter()
        .map(|b| ButtonRow {
            button: b,
            normal: find(normal, b),
            hypershift: find(hypershift, b),
        })
        .collect()
}

/// Everything the panel shows.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub firmware: FirmwareVersion,
    pub serial: Field<String>,
    pub mode: Field<(DeviceMode, u8)>,
    pub battery: Field<BatteryLevel>,
    pub charging: Field<ChargingStatus>,
    pub dpi: Field<dpi::Dpi>,
    pub stages: Field<dpi::DpiStages>,
    pub polling: Field<(u32, PollingVariant)>,
    pub sleep: Field<u16>,
    pub buttons_normal: Field<Vec<ButtonAssignment>>,
    pub buttons_hypershift: Field<Vec<ButtonAssignment>>,
}

/// Read the whole panel. The firmware read goes first: if the device does not
/// answer it, give up at once instead of retrying every other field.
pub fn read_snapshot<D: FeatureIo>(s: &mut Session<D>) -> Result<Snapshot, String> {
    let firmware = query(
        s,
        &device::get_firmware_version(),
        device::parse_firmware_version,
    )
    .map_err(|e| format!("device does not answer: {e}"))?;

    Ok(Snapshot {
        firmware,
        serial: f(query(s, &device::get_serial(), device::parse_serial)),
        mode: f(query(
            s,
            &device::get_device_mode(),
            device::parse_device_mode,
        )),
        battery: f(query(
            s,
            &power::get_battery_level(),
            power::parse_battery_level,
        )),
        charging: f(query(
            s,
            &power::get_charging_status(),
            power::parse_charging_status,
        )),
        dpi: f(query(
            s,
            &dpi::get_dpi(dpi::Storage::Volatile),
            dpi::parse_dpi,
        )),
        stages: f(query(
            s,
            &dpi::get_dpi_stages(dpi::Storage::Volatile),
            dpi::parse_dpi_stages,
        )),
        polling: f(read_polling(s)),
        sleep: f(query(s, &power::get_sleep_time(), power::parse_sleep_time)),
        buttons_normal: f(read_button_list(s, Mode::Normal)),
        buttons_hypershift: f(read_button_list(s, Mode::HyperShift)),
    })
}

/// What the action editor offers for one button slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// Plain mouse button (one of [`MOUSE_TARGETS`]).
    Mouse(u8),
    /// Double left click (data `[1]`, the only hardware-verified value).
    DoubleClick,
    /// Keyboard key with modifiers.
    Key,
    /// Step DPI up.
    DpiUp,
    /// Anything else the device reports; shown, never written by the editor.
    Other,
}

impl ActionKind {
    /// The choices of the action combo, in display order ([`Self::Other`] last).
    pub fn choices() -> Vec<ActionKind> {
        MOUSE_TARGETS
            .map(ActionKind::Mouse)
            .chain([Self::DoubleClick, Self::Key, Self::DpiUp, Self::Other])
            .collect()
    }

    pub fn label(self) -> String {
        match self {
            Self::Mouse(1) => "Left click".into(),
            Self::Mouse(2) => "Right click".into(),
            Self::Mouse(3) => "Middle click".into(),
            Self::Mouse(n) => format!("Mouse button {n}"),
            Self::DoubleClick => "Double-click".into(),
            Self::Key => "Key".into(),
            Self::DpiUp => "DPI up".into(),
            Self::Other => "Other (read-only)".into(),
        }
    }
}

/// Editor state of one button slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionEdit {
    pub kind: ActionKind,
    /// HID usage of [`ActionKind::Key`] (kept while another kind is chosen).
    pub usage: u8,
    /// HID boot-keyboard modifier bits of [`ActionKind::Key`].
    pub modifiers: u8,
}

/// Key `a` until the user picks another.
const DEFAULT_USAGE: u8 = 0x04;

impl ActionEdit {
    /// Start the editor at what the device reports.
    pub fn from_assignment(a: &ButtonAssignment) -> Self {
        let mut e = Self {
            kind: ActionKind::Other,
            usage: DEFAULT_USAGE,
            modifiers: 0,
        };
        match a.known() {
            Some(Known::MouseButton(t)) if MOUSE_TARGETS.contains(&t) => {
                e.kind = ActionKind::Mouse(t);
            }
            Some(Known::DoubleClick(1)) => e.kind = ActionKind::DoubleClick,
            Some(Known::Key { modifiers, usage })
                if hid_usage::name(usage).is_some()
                    && hid_usage::modifier_names(modifiers).is_some() =>
            {
                e = Self {
                    kind: ActionKind::Key,
                    usage,
                    modifiers,
                };
            }
            Some(Known::DpiUp) => e.kind = ActionKind::DpiUp,
            _ => {}
        }
        e
    }

    /// The assignment to write, built by the protocol builders; `None` for
    /// [`ActionKind::Other`].
    pub fn to_assignment(self, button: u8, mode: Mode) -> Option<ButtonAssignment> {
        let p = BUTTON_PROFILE;
        Some(match self.kind {
            ActionKind::Mouse(t) => buttons::mouse_button(p, button, mode, t),
            ActionKind::DoubleClick => buttons::double_click(p, button, mode, 1),
            ActionKind::Key => buttons::keyboard(p, button, mode, self.modifiers, self.usage),
            ActionKind::DpiUp => buttons::dpi_up(p, button, mode),
            ActionKind::Other => return None,
        })
    }
}

/// Keys the editor offers: every usage `hid_usage` can name, as `(usage, name)`.
pub fn key_choices() -> Vec<(u8, String)> {
    (0..=u8::MAX)
        .filter_map(|u| hid_usage::name(u).map(|n| (u, n)))
        .collect()
}

/// Modifier checkboxes: `(bit, label)` with bits from `hid_usage`.
pub fn modifier_choices() -> Vec<(u8, &'static str)> {
    [
        ("ctrl", "Ctrl"),
        ("shift", "Shift"),
        ("alt", "Alt"),
        ("win", "Win"),
    ]
    .into_iter()
    .filter_map(|(n, label)| hid_usage::modifier(n).map(|b| (b, label)))
    .collect()
}

/// Round `v` to the registry grid `min + k·step` inside `min..=max`.
pub fn snap_dpi(v: u16, min: u16, max: u16, step: u16) -> u16 {
    let step = step.max(1);
    let v = v.clamp(min, max);
    let k = (u32::from(v - min) + u32::from(step) / 2) / u32::from(step);
    let snapped = u32::from(min) + k * u32::from(step);
    if snapped > u32::from(max) {
        // Round down instead of past the top.
        min + (v - min) / step * step
    } else {
        snapped as u16
    }
}

/// Most stages the editor allows: at least what the device reports now, at least
/// [`DEFAULT_MAX_STAGES`], never more than the protocol fits.
pub fn stage_limit(current: usize) -> usize {
    current.clamp(DEFAULT_MAX_STAGES, dpi::MAX_DPI_STAGES)
}

/// A stage list from editor values. Stage IDs are 1-based positions (as read from
/// the device and as the core profile plan writes them); `active` is a 0-based
/// position.
pub fn stages_from(values: &[u16], active: usize) -> Result<dpi::DpiStages, String> {
    if values.is_empty() || values.len() > dpi::MAX_DPI_STAGES {
        return Err(format!(
            "need 1-{} stages, got {}",
            dpi::MAX_DPI_STAGES,
            values.len()
        ));
    }
    let stages: Vec<dpi::DpiStage> = values
        .iter()
        .zip(1u8..)
        .map(|(&v, id)| dpi::DpiStage {
            id,
            dpi: dpi::Dpi::uniform(v),
        })
        .collect();
    let all = dpi::DpiStages { active: 0, stages };
    all.with_active(active).map_err(|e| e.to_string())
}

/// Options the user set for a write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteOpts {
    /// Razer software runs and the user ticked "allow writes anyway".
    pub confirmed: bool,
    /// The user ticked "allow editing left/right click".
    pub allow_primary: bool,
    /// Writing back a value just read from the device ("Restore"). Skips the
    /// left/right-click guard and the known-action check: the bytes are exactly
    /// what the device held before.
    pub restore: bool,
}

/// Actions the editor may write (known layout, values the GUI offers).
fn check_action(a: &ButtonAssignment) -> Result<(), String> {
    let ok = match a.known() {
        Some(Known::MouseButton(t)) => MOUSE_TARGETS.contains(&t),
        Some(Known::DoubleClick(t)) => t == 1,
        Some(Known::Key { modifiers, usage }) => {
            hid_usage::name(usage).is_some() && hid_usage::modifier_names(modifiers).is_some()
        }
        Some(Known::DpiUp) => true,
        None => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{} is not an action the GUI writes (mouse button 1-5, double-click, \
             named key, DPI up)",
            action_text(a)
        ))
    }
}

/// Reject values outside what the registry says the device supports and button
/// writes the user did not allow. Unknown devices get no writes at all (the GUI
/// has no `--force`).
pub fn validate(new: &Setting, device: Option<&Device>, opts: WriteOpts) -> Result<(), String> {
    let Some(d) = device else {
        return Err("device is not in the registry; writes are disabled".into());
    };
    let dpi_ok = |v: dpi::Dpi| -> Result<(), String> {
        for x in [v.x, v.y] {
            if !d.dpi_valid(u32::from(x)) {
                return Err(format!(
                    "{x} DPI is outside {}-{} (step {}) for this device",
                    d.dpi_min.unwrap_or(0),
                    d.dpi_max.unwrap_or(0),
                    d.dpi_step.unwrap_or(1)
                ));
            }
        }
        Ok(())
    };
    match new {
        Setting::Dpi(v) => dpi_ok(*v)?,
        Setting::Polling { hz, .. } => {
            let offered: &[u32] = if d.polling_rates.is_empty() {
                &FALLBACK_POLLING_RATES
            } else {
                &d.polling_rates
            };
            if !offered.contains(hz) {
                return Err(format!(
                    "{hz} Hz is not offered for this device ({offered:?})"
                ));
            }
        }
        Setting::Sleep(0) => return Err("sleep timer 0 is untested; refusing".into()),
        Setting::Sleep(_) => {}
        Setting::Button(a) => {
            if !opts.restore {
                if PRIMARY_BUTTONS.contains(&a.button) && !opts.allow_primary {
                    return Err(format!(
                        "{} is protected; tick \"allow editing left/right click\" first",
                        button_name(a.button)
                    ));
                }
                check_action(a)?;
            }
        }
        Setting::DpiStages { stages, .. } => {
            let n = stages.stages.len();
            if n == 0 || n > dpi::MAX_DPI_STAGES {
                return Err(format!("need 1-{} stages, got {n}", dpi::MAX_DPI_STAGES));
            }
            for st in &stages.stages {
                dpi_ok(st.dpi)?;
            }
            let mut ids: Vec<u8> = stages.stages.iter().map(|s| s.id).collect();
            ids.sort_unstable();
            ids.dedup();
            if ids.len() != n {
                return Err("stage IDs are not unique".into());
            }
            if stages.active_index().is_none() {
                return Err(format!(
                    "active stage ID {} matches no stage",
                    stages.active
                ));
            }
        }
    }
    Ok(())
}

/// Writes are refused while Razer software runs unless the user confirmed.
pub fn conflict_gate(conflict: Option<&str>, confirmed: bool) -> Result<(), String> {
    match conflict {
        Some(name) if !confirmed => Err(format!(
            "{name} is running and may talk to the device at the same time; \
             close it or tick the confirmation box"
        )),
        _ => Ok(()),
    }
}

/// A finished write: the value before, the value asked for, the value read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteOutcome {
    pub old: Setting,
    pub new: Setting,
    pub now: Setting,
}

impl WriteOutcome {
    pub fn verified(&self) -> bool {
        self.now == self.new
    }
}

/// A write that did not finish. `old` is set when the old value was read, so the
/// UI can still offer to restore it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteFailure {
    pub old: Option<Setting>,
    pub message: String,
}

impl WriteFailure {
    fn before_read(message: String) -> Self {
        Self { old: None, message }
    }
}

/// Validate → conflict gate → read old → write → read back.
pub fn guarded_write<D: FeatureIo>(
    s: &mut Session<D>,
    new: Setting,
    device: Option<&Device>,
    conflict: Option<&str>,
    opts: WriteOpts,
) -> Result<WriteOutcome, WriteFailure> {
    validate(&new, device, opts).map_err(WriteFailure::before_read)?;
    conflict_gate(conflict, opts.confirmed).map_err(WriteFailure::before_read)?;
    let req = new
        .write_request()
        .map_err(|e| WriteFailure::before_read(e.to_string()))?;
    let old = new
        .read_current(s)
        .map_err(|e| WriteFailure::before_read(format!("cannot read the current value: {e}")))?;
    let fail = |what: &str, e: HidError| WriteFailure {
        old: Some(old.clone()),
        message: format!("{what}: {e}"),
    };
    s.execute(&req).map_err(|e| fail("write failed", e))?;
    let now = new
        .read_current(s)
        .map_err(|e| fail("read-back failed", e))?;
    Ok(WriteOutcome { old, new, now })
}

pub mod profile_apply;

#[cfg(any(test, feature = "fake"))]
pub mod fake;

#[cfg(test)]
mod tests;
