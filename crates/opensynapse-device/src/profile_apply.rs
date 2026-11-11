//! Profile export and apply: device state <-> `opensynapse_core::Profile`.
//!
//! The one apply engine, shared by `opensynapse profile apply` (direct HID) and the
//! agent's `ApplyProfile` request. Generic over [`FeatureIo`]; no printing, no file
//! I/O: callers get lines and a restore [`Profile`] and decide where they go.
//!
//! Apply = validate → read a snapshot of exactly what the profile overwrites →
//! plan (`Profile::plan`) → skip writes whose setting already has the target value
//! → (caller saves the restore profile) → write each, read it back, compare.
//! Requests come only from `opensynapse-protocol` builders (directly or through
//! `Profile::plan`).

use std::fmt::Write as _;

use opensynapse_core::validate::{MAX_MOUSE_BUTTON, POLLING_RATES};
use opensynapse_core::{
    ButtonAction, ButtonId, DeviceLimits, DeviceSection, DpiSection, HyperShiftSection, Pid,
    PlannedWrite, PollingSection, PollingVariant, PowerSection, Profile, Setting, Target,
    TomlError, ValidationError,
};
use opensynapse_device_registry::Device;
use opensynapse_hid::{FeatureIo, HidError, Session};
use opensynapse_protocol::buttons::{self, ButtonAssignment, Known, Mode};
use opensynapse_protocol::{ProtocolError, Request, Status, dpi, hid_usage, polling, power};

use crate::query;

/// Onboard profile slot that export reads and apply writes (button assignments;
/// also the HyperPolling profile byte).
pub const ONBOARD_PROFILE: u8 = 1;

/// Raw device values, as read. `None` / missing entries were not read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceState {
    pub pid: u16,
    pub stages: Option<dpi::DpiStages>,
    pub polling_hz: Option<u32>,
    pub sleep: Option<u16>,
    pub buttons: Vec<ButtonAssignment>,
}

/// A profile built from a [`DeviceState`] plus notes about values it had to leave out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub profile: Profile,
    pub notes: Vec<String>,
}

/// Why export or apply stopped.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    /// The profile does not pass validation against the device limits.
    #[error("{0}")]
    Invalid(ValidationError),
    #[error(
        "the profile is for PID {profile}, but the device is {device:04X}; use --force to apply anyway"
    )]
    WrongDevice { profile: Pid, device: u16 },
    #[error("{hz} Hz is not offered for this device ({offered:?}); use --force to override")]
    PollingNotOffered { hz: u32, offered: Vec<u32> },
    /// A snapshot read failed (strict reads only).
    #[error("cannot read {what}")]
    Read {
        what: String,
        #[source]
        source: HidError,
    },
    #[error(transparent)]
    Plan(ValidationError),
    /// The restore profile could not undo the plan; nothing was written.
    #[error(
        "the snapshot taken before writing could not undo this profile:\n  {}\n\
         Nothing was written. Use --force to apply anyway.",
        .0.join("\n  ")
    )]
    SnapshotIncomplete(Vec<String>),
    /// Razer software runs and the write was not confirmed (`force`).
    #[error(
        "{0} is running and may talk to the device at the same time; close it or confirm (--force)"
    )]
    Conflict(String),
    /// Write `index` (0-based) of `total` failed; earlier writes were sent.
    #[error("{source}")]
    Write {
        index: usize,
        total: usize,
        setting: Setting,
        source: WriteError,
    },
}

impl ApplyError {
    /// `Stopped at write 2 of 3 (…); later writes were not sent.` for a write failure.
    pub fn stopped_line(&self) -> Option<String> {
        match self {
            Self::Write {
                index,
                total,
                setting,
                ..
            } => Some(format!(
                "Stopped at write {} of {total} ({setting}); later writes were not sent.",
                index + 1
            )),
            _ => None,
        }
    }
}

/// Why one planned write failed.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error(transparent)]
    Hid(#[from] HidError),
    #[error("{setting} reads back as {now}, not the value written")]
    Mismatch { setting: Setting, now: String },
}

impl From<ProtocolError> for WriteError {
    fn from(e: ProtocolError) -> Self {
        Self::Hid(e.into())
    }
}

/// Express `state` as a profile. Values the profile model cannot hold are left out
/// and listed (raw) in `notes`.
pub fn export_profile(state: &DeviceState) -> Export {
    let mut notes = Vec::new();
    let mut p = Profile {
        device: Some(DeviceSection {
            pid: Pid(state.pid),
        }),
        ..Profile::default()
    };
    if let Some(s) = &state.stages {
        match dpi_section(s) {
            Ok(d) => p.dpi = Some(d),
            Err(why) => notes.push(format!("DPI stages left out ({why}): raw {s:?}")),
        }
    }
    if let Some(hz) = state.polling_hz {
        if POLLING_RATES.contains(&hz) {
            p.polling = Some(PollingSection { hz });
        } else {
            notes.push(format!(
                "polling rate {hz} Hz left out: profiles support {POLLING_RATES:?} Hz"
            ));
        }
    }
    if let Some(v) = state.sleep {
        p.power = Some(PowerSection { sleep_seconds: v });
    }
    let mut hs = HyperShiftSection::default();
    for a in &state.buttons {
        let Some(action) = button_action(a) else {
            notes.push(format!(
                "button {} ({} mode) left out: function {}, data {:?} cannot be expressed",
                a.button, a.mode, a.function, a.data
            ));
            continue;
        };
        let mode = match a.mode {
            Mode::Normal => &mut p.buttons,
            Mode::HyperShift => &mut hs.buttons,
        };
        mode.insert(ButtonId(a.button), action);
    }
    if !hs.buttons.is_empty() {
        p.hypershift = Some(hs);
    }
    Export { profile: p, notes }
}

/// Drop each `[hypershift.buttons.N]` entry that equals `[buttons.N]`, so an export
/// lists only HyperShift mappings that differ from the normal one. Only for export:
/// the restore snapshot keeps every slot it read, because it must undo exactly what
/// apply overwrites.
pub fn drop_redundant_hypershift(p: &mut Profile) {
    if let Some(hs) = &mut p.hypershift {
        hs.buttons
            .retain(|id, action| p.buttons.get(id) != Some(&*action));
        if hs.buttons.is_empty() {
            p.hypershift = None;
        }
    }
}

/// Stages as a profile section: stage IDs must be the positions 1, 2, … (that is
/// what `Profile::plan` writes back) and X must equal Y.
fn dpi_section(s: &dpi::DpiStages) -> Result<DpiSection, String> {
    let mut stages = Vec::with_capacity(s.stages.len());
    for (i, st) in s.stages.iter().enumerate() {
        if usize::from(st.id) != i + 1 {
            return Err(format!("stage at position {} has ID {}", i + 1, st.id));
        }
        if st.dpi.x != st.dpi.y {
            return Err(format!("stage {} has different X/Y DPI {}", i + 1, st.dpi));
        }
        stages.push(st.dpi.x);
    }
    let i = s
        .active_index()
        .ok_or_else(|| format!("active stage ID {} matches no stage", s.active))?;
    let active_stage = u8::try_from(i + 1).map_err(|_| "too many stages".to_string())?;
    Ok(DpiSection {
        stages,
        active_stage,
    })
}

/// The profile action for an assignment, or `None` if the model cannot express it.
fn button_action(a: &ButtonAssignment) -> Option<ButtonAction> {
    let mouse = |t: u8| (1..=MAX_MOUSE_BUTTON).contains(&t);
    Some(match a.known()? {
        Known::MouseButton(t) if mouse(t) => ButtonAction::MouseButton { target: t },
        Known::DoubleClick(t) if mouse(t) => ButtonAction::DoubleClick { target: t },
        Known::Key { modifiers, usage } => ButtonAction::Key {
            key: hid_usage::name(usage)?,
            mods: hid_usage::modifier_names(modifiers)?
                .into_iter()
                .map(String::from)
                .collect(),
        },
        Known::DpiUp => ButtonAction::DpiUp {},
        Known::DpiDown => ButtonAction::DpiDown {},
        Known::Off => ButtonAction::Off {},
        Known::MouseButton(_) | Known::DoubleClick(_) => return None,
    })
}

/// Profile text with a comment header.
pub fn render(profile: &Profile, header: &str) -> Result<String, TomlError> {
    let mut out = String::new();
    for line in header.lines() {
        let _ = writeln!(out, "# {line}");
    }
    out.push_str(&profile.to_toml()?);
    Ok(out)
}

/// `OpenSynapse profile: {what} {name} ({pid}), onboard profile 1.`
pub fn header(name: &str, pid: u16, what: &str) -> String {
    format!("OpenSynapse profile: {what} {name} ({pid:04X}), onboard profile {ONBOARD_PROFILE}.")
}

/// Device limits from the registry entry; unknown values fall back to "anything the
/// protocol allows". The registry has no stage-count limit yet, so that one always
/// falls back to the protocol maximum.
pub fn limits_for(device: Option<&Device>) -> DeviceLimits {
    let mut l = DeviceLimits::default();
    if let Some(d) = device {
        let u = |v: Option<u32>| v.and_then(|v| u16::try_from(v).ok());
        l.dpi_min = u(d.dpi_min).unwrap_or(l.dpi_min);
        l.dpi_max = u(d.dpi_max).unwrap_or(l.dpi_max);
        l.dpi_step = u(d.dpi_step).unwrap_or(l.dpi_step);
    }
    l
}

/// Why the snapshot cannot undo `plan`: a setting the plan overwrites is missing from
/// it (its old value could not be expressed), or it would not pass validation.
pub fn snapshot_problems(
    plan: &[PlannedWrite],
    snap: &Profile,
    limits: &DeviceLimits,
) -> Vec<String> {
    let mut out: Vec<String> = plan
        .iter()
        .filter(|w| setting_value(snap, w.setting).is_none())
        .map(|w| {
            format!(
                "{}: its current value cannot be saved in a profile, so it could not be restored",
                w.setting
            )
        })
        .collect();
    if let Err(e) = snap.validate(limits) {
        out.push(format!("the restore file would not pass validation: {e}"));
    }
    out
}

/// What `p` sets for `setting`, for display; `None` if it does not set it.
pub fn setting_value(p: &Profile, setting: Setting) -> Option<String> {
    match setting {
        Setting::DpiStages => p.dpi.as_ref().map(|d| {
            let s: Vec<String> = d.stages.iter().map(u16::to_string).collect();
            format!("[{}] (active {})", s.join(", "), d.active_stage)
        }),
        Setting::PollingRate => p.polling.as_ref().map(|x| format!("{} Hz", x.hz)),
        Setting::SleepTime => p.power.as_ref().map(|x| format!("{} s", x.sleep_seconds)),
        Setting::Button { mode, button } => {
            let map = match mode {
                Mode::Normal => Some(&p.buttons),
                Mode::HyperShift => p.hypershift.as_ref().map(|h| &h.buttons),
            };
            map.and_then(|m| m.get(&ButtonId(button))).map(action_label)
        }
    }
}

fn action_label(a: &ButtonAction) -> String {
    match a {
        ButtonAction::MouseButton { target } => format!("mouse button {target}"),
        ButtonAction::DoubleClick { target } => format!("double-click {target}"),
        ButtonAction::Key { key, mods } if mods.is_empty() => format!("key {key}"),
        ButtonAction::Key { key, mods } => format!("key {} {key}", mods.join("+")),
        ButtonAction::DpiUp {} => "DPI up".into(),
        ButtonAction::DpiDown {} => "DPI down".into(),
        ButtonAction::Off {} => "off".into(),
    }
}

/// A plan split into the writes to send and the ones whose setting already has the
/// target value, plus the snapshot that undoes `writes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub writes: Vec<PlannedWrite>,
    pub unchanged: Vec<PlannedWrite>,
    /// Old values of exactly the settings in `writes`; `None` when nothing is written
    /// (then no restore file is needed).
    pub snapshot: Option<Export>,
}

/// Split `plan` against the values read in `state`. A write is unchanged when
/// re-encoding the current value gives the same request (command + arguments; the
/// transaction ID is not part of a `Request`). A setting that was not read, or whose
/// current value cannot be encoded, counts as changed. `all` keeps every write.
pub fn prepare(
    plan: Vec<PlannedWrite>,
    state: &DeviceState,
    target: &Target,
    all: bool,
) -> Prepared {
    let (writes, unchanged): (Vec<_>, Vec<_>) = plan.into_iter().partition(|w| {
        all || current_request(state, w.setting, target).as_ref() != Some(&w.request)
    });
    let snapshot = (!writes.is_empty()).then(|| export_profile(&state_subset(state, &writes)));
    Prepared {
        writes,
        unchanged,
        snapshot,
    }
}

/// The set request for `hz` in variant `v` (`profile` is the HyperPolling profile byte).
fn polling_request(v: PollingVariant, profile: u8, hz: u32) -> Result<Request, ProtocolError> {
    match v {
        PollingVariant::Period => polling::set_polling_rate(hz),
        PollingVariant::Hyper => polling::set_hyper_polling(profile, hz),
    }
}

/// The request that would write the value `state` holds for `setting`, if it was
/// read and can be encoded. Same rebuild as [`read_back`].
fn current_request(state: &DeviceState, setting: Setting, target: &Target) -> Option<Request> {
    match setting {
        Setting::DpiStages => dpi::set_dpi_stages(target.dpi_storage, state.stages.as_ref()?).ok(),
        Setting::PollingRate => {
            polling_request(target.polling, target.onboard_profile, state.polling_hz?).ok()
        }
        Setting::SleepTime => Some(power::set_sleep_time(state.sleep?)),
        Setting::Button { mode, button } => state
            .buttons
            .iter()
            .find(|a| a.mode == mode && a.button == button)
            .and_then(|a| buttons::set_button_assignment(a).ok()),
    }
}

/// Only the values of `state` that `writes` overwrite.
fn state_subset(state: &DeviceState, writes: &[PlannedWrite]) -> DeviceState {
    let has = |s: Setting| writes.iter().any(|w| w.setting == s);
    DeviceState {
        pid: state.pid,
        stages: state.stages.clone().filter(|_| has(Setting::DpiStages)),
        polling_hz: state.polling_hz.filter(|_| has(Setting::PollingRate)),
        sleep: state.sleep.filter(|_| has(Setting::SleepTime)),
        buttons: state
            .buttons
            .iter()
            .filter(|a| {
                has(Setting::Button {
                    mode: a.mode,
                    button: a.button,
                })
            })
            .cloned()
            .collect(),
    }
}

/// What to read from the device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadSpec {
    pub dpi: Option<dpi::Storage>,
    pub polling: bool,
    pub sleep: bool,
    pub buttons: Vec<(Mode, u8)>,
}

impl ReadSpec {
    /// Everything export covers: persistent DPI stages, polling, sleep, and `ids` in
    /// normal mode, then in HyperShift mode.
    pub fn export(ids: &[u8]) -> Self {
        Self {
            dpi: Some(dpi::Storage::Persistent),
            polling: true,
            sleep: true,
            buttons: [Mode::Normal, Mode::HyperShift]
                .iter()
                .flat_map(|&m| ids.iter().map(move |&b| (m, b)))
                .collect(),
        }
    }

    /// Exactly what applying `p` overwrites.
    pub fn for_profile(p: &Profile, storage: dpi::Storage) -> Self {
        Self {
            dpi: p.dpi.as_ref().map(|_| storage),
            polling: p.polling.is_some(),
            sleep: p.power.is_some(),
            buttons: p.all_buttons().map(|(l, id, _)| (l, id.0)).collect(),
        }
    }
}

/// Polling rate in `variant`, or auto: the period variant, falling back to
/// HyperPolling (profile byte `profile`) when the device rejects it.
fn read_polling_at<D: FeatureIo>(
    s: &mut Session<D>,
    variant: Option<PollingVariant>,
    profile: u8,
) -> Result<(u32, PollingVariant), HidError> {
    let period =
        |s: &mut Session<D>| query(s, &polling::get_polling_rate(), polling::parse_polling_rate);
    let hyper = |s: &mut Session<D>| {
        query(
            s,
            &polling::get_hyper_polling(profile),
            polling::parse_hyper_polling,
        )
    };
    match variant {
        Some(PollingVariant::Period) => Ok((period(s)?, PollingVariant::Period)),
        Some(PollingVariant::Hyper) => Ok((hyper(s)?, PollingVariant::Hyper)),
        None => match period(s) {
            Ok(hz) => Ok((hz, PollingVariant::Period)),
            Err(HidError::Status(
                Status::NotSupported | Status::UnknownCommand | Status::WrongDeviceMode,
            )) => Ok((hyper(s)?, PollingVariant::Hyper)),
            Err(e) => Err(e),
        },
    }
}

fn read_button<D: FeatureIo>(
    s: &mut Session<D>,
    button: u8,
    mode: Mode,
) -> Result<buttons::ButtonReply, HidError> {
    query(
        s,
        &buttons::get_button_assignment(ONBOARD_PROFILE, button, mode),
        |args| buttons::parse_button_assignment(args, mode),
    )
}

/// Read what `spec` lists for device `pid`. `lenient`: a failed read becomes a note
/// instead of an error. Also returns the polling variant the device answered (if
/// polling was read).
pub fn read_state<D: FeatureIo>(
    s: &mut Session<D>,
    pid: u16,
    spec: &ReadSpec,
    lenient: bool,
) -> Result<(DeviceState, Option<PollingVariant>, Vec<String>), ApplyError> {
    let mut notes = Vec::new();
    let mut state = DeviceState {
        pid,
        ..DeviceState::default()
    };
    if let Some(storage) = spec.dpi {
        let r = query(s, &dpi::get_dpi_stages(storage), dpi::parse_dpi_stages);
        state.stages = soft(&mut notes, lenient, "DPI stages".into(), r)?;
    }
    let mut variant = None;
    if spec.polling {
        // Same auto detection as `polling get|set`: period, then HyperPolling.
        let r = read_polling_at(s, None, ONBOARD_PROFILE);
        if let Some((hz, v)) = soft(&mut notes, lenient, "polling rate".into(), r)? {
            state.polling_hz = Some(hz);
            variant = Some(v);
        }
    }
    if spec.sleep {
        let r = query(s, &power::get_sleep_time(), power::parse_sleep_time);
        state.sleep = soft(&mut notes, lenient, "sleep time".into(), r)?;
    }
    for &(mode, b) in &spec.buttons {
        let r = read_button(s, b, mode);
        if let Some(r) = soft(&mut notes, lenient, format!("button {b} ({mode} mode)"), r)? {
            state.buttons.push(r.assignment);
        }
    }
    Ok((state, variant, notes))
}

/// A read result: `lenient` turns an error into a note.
fn soft<T>(
    notes: &mut Vec<String>,
    lenient: bool,
    what: String,
    r: Result<T, HidError>,
) -> Result<Option<T>, ApplyError> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if lenient => {
            notes.push(format!("{what} not read: {e}"));
            Ok(None)
        }
        Err(source) => Err(ApplyError::Read { what, source }),
    }
}

/// Export: read (leniently) DPI stages, polling, sleep and buttons `ids`, as a
/// profile without redundant HyperShift entries. `notes` lists failed reads and
/// values left out.
pub fn export<D: FeatureIo>(
    s: &mut Session<D>,
    pid: u16,
    ids: &[u8],
) -> Result<Export, ApplyError> {
    let (state, _, mut notes) = read_state(s, pid, &ReadSpec::export(ids), true)?;
    let mut ex = export_profile(&state);
    drop_redundant_hypershift(&mut ex.profile);
    notes.extend(ex.notes);
    ex.notes = notes;
    Ok(ex)
}

/// How to apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyOptions {
    /// Where DPI stages go.
    pub storage: dpi::Storage,
    /// Also write settings that already have the profile's value.
    pub all: bool,
    /// Skip the PID, offered-rate and snapshot-completeness checks (and, in
    /// [`apply`], the Razer-software conflict gate).
    pub force: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            storage: dpi::Storage::Persistent,
            all: false,
            force: false,
        }
    }
}

/// A checked, planned apply: what will be written, what is skipped, and the
/// profile that undoes it. Nothing has been written yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyPlan {
    pub profile: Profile,
    pub target: Target,
    pub writes: Vec<PlannedWrite>,
    pub unchanged: Vec<PlannedWrite>,
    /// Old values of exactly the settings in `writes` (`None` when nothing is
    /// written), with notes about values it had to leave out.
    pub snapshot: Option<Export>,
    /// Why the snapshot could not undo `writes` (see [`snapshot_problems`]).
    pub problems: Vec<String>,
    pub force: bool,
}

impl ApplyPlan {
    /// The profile plans no writes at all.
    pub fn sets_nothing(&self) -> bool {
        self.writes.is_empty() && self.unchanged.is_empty()
    }

    /// The profile that undoes `writes`, if anything is written.
    pub fn restore(&self) -> Option<&Profile> {
        self.snapshot.as_ref().map(|s| &s.profile)
    }

    /// `unchanged: DPI stages ([800, 1600] (active 1))`, one per skipped write.
    pub fn unchanged_lines(&self) -> Vec<String> {
        self.unchanged
            .iter()
            .map(|w| {
                let v = setting_value(&self.profile, w.setting).unwrap_or_else(|| "?".into());
                format!("unchanged: {} ({v})", w.setting)
            })
            .collect()
    }

    /// `2 write(s), 1 unchanged.`
    pub fn summary_line(&self) -> String {
        format!(
            "{} write(s), {} unchanged.",
            self.writes.len(),
            self.unchanged.len()
        )
    }

    /// `polling rate: 1000 Hz -> 500 Hz`.
    pub fn change(&self, w: &PlannedWrite) -> String {
        let old = self
            .restore()
            .and_then(|r| setting_value(r, w.setting))
            .unwrap_or_else(|| "?".into());
        let new = setting_value(&self.profile, w.setting).unwrap_or_else(|| "?".into());
        format!("{}: {old} -> {new}", w.setting)
    }

    /// What a dry run shows: each change with its report, then the refusals.
    pub fn dry_run_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for w in &self.writes {
            out.push(format!("Dry run: would change {}.", self.change(w)));
            out.push(format!("Report: {:?}", w.request.encode(0)));
        }
        for p in &self.problems {
            out.push(format!("Refusal (--apply needs --force): {p}"));
        }
        out
    }

    /// Refuse to write when the snapshot could not undo the plan (unless forced).
    pub fn check_snapshot(&self) -> Result<(), ApplyError> {
        if self.problems.is_empty() || self.force {
            Ok(())
        } else {
            Err(ApplyError::SnapshotIncomplete(self.problems.clone()))
        }
    }
}

/// Validate `profile` for the device, read the snapshot of what it overwrites and
/// split the plan into writes and unchanged settings. Reads only.
pub fn plan_apply<D: FeatureIo>(
    s: &mut Session<D>,
    pid: u16,
    device: Option<&Device>,
    profile: Profile,
    opts: ApplyOptions,
) -> Result<ApplyPlan, ApplyError> {
    let limits = limits_for(device);
    profile.validate(&limits).map_err(ApplyError::Invalid)?;
    if let Some(d) = &profile.device
        && d.pid.0 != pid
        && !opts.force
    {
        return Err(ApplyError::WrongDevice {
            profile: d.pid,
            device: pid,
        });
    }
    if let (Some(p), Some(d)) = (&profile.polling, device)
        && !d.polling_rates.is_empty()
        && !d.polling_rates.contains(&p.hz)
        && !opts.force
    {
        return Err(ApplyError::PollingNotOffered {
            hz: p.hz,
            offered: d.polling_rates.clone(),
        });
    }
    // Snapshot of everything the profile overwrites (also detects the polling variant).
    let (state, variant, _) = read_state(
        s,
        pid,
        &ReadSpec::for_profile(&profile, opts.storage),
        false,
    )?;
    let target = Target {
        limits,
        onboard_profile: ONBOARD_PROFILE,
        dpi_storage: opts.storage,
        polling: variant.unwrap_or_default(),
    };
    let plan = profile.plan(&target).map_err(ApplyError::Plan)?;
    let Prepared {
        writes,
        unchanged,
        snapshot,
    } = prepare(plan, &state, &target, opts.all);
    let problems = snapshot
        .as_ref()
        .map(|snap| snapshot_problems(&writes, &snap.profile, &limits))
        .unwrap_or_default();
    Ok(ApplyPlan {
        profile,
        target,
        writes,
        unchanged,
        snapshot,
        problems,
        force: opts.force,
    })
}

/// Send the planned writes in order; each is read back and compared. `line` gets
/// `Verified: …` and `Changed …` as they happen. Stops at the first failure
/// ([`ApplyError::Write`]). Does not check the snapshot or the conflict gate: call
/// [`ApplyPlan::check_snapshot`] (and save the restore profile) first.
pub fn execute<D: FeatureIo>(
    s: &mut Session<D>,
    plan: &ApplyPlan,
    mut line: impl FnMut(String),
) -> Result<(), ApplyError> {
    let total = plan.writes.len();
    for (index, w) in plan.writes.iter().enumerate() {
        match write_one(s, w, &plan.target) {
            Ok(now) => {
                line(format!("Verified: {} is now {now}.", w.setting));
                line(format!("Changed {}.", plan.change(w)));
            }
            Err(source) => {
                return Err(ApplyError::Write {
                    index,
                    total,
                    setting: w.setting,
                    source,
                });
            }
        }
    }
    Ok(())
}

/// Send one planned write, read the setting back and check that re-encoding the
/// read-back value gives the same request. Returns the read-back value for display.
fn write_one<D: FeatureIo>(
    s: &mut Session<D>,
    w: &PlannedWrite,
    target: &Target,
) -> Result<String, WriteError> {
    s.execute(&w.request)?;
    let (now, rebuilt) = read_back(s, w.setting, target)?;
    if rebuilt != w.request {
        return Err(WriteError::Mismatch {
            setting: w.setting,
            now,
        });
    }
    Ok(now)
}

/// 1-based position of the active stage, or the raw ID if no stage has it.
fn active_label(s: &dpi::DpiStages) -> String {
    match s.active_index() {
        Some(i) => (i + 1).to_string(),
        None => format!("? (ID {} matches no stage)", s.active),
    }
}

/// The current value of `setting` (for display) and the request that would write it.
fn read_back<D: FeatureIo>(
    s: &mut Session<D>,
    setting: Setting,
    target: &Target,
) -> Result<(String, Request), WriteError> {
    let p = target.onboard_profile;
    Ok(match setting {
        Setting::DpiStages => {
            let st = query(
                s,
                &dpi::get_dpi_stages(target.dpi_storage),
                dpi::parse_dpi_stages,
            )?;
            let list: Vec<String> = st.stages.iter().map(|x| x.dpi.to_string()).collect();
            let label = format!("[{}] (active {})", list.join(", "), active_label(&st));
            (label, dpi::set_dpi_stages(target.dpi_storage, &st)?)
        }
        Setting::PollingRate => {
            let (hz, _) = read_polling_at(s, Some(target.polling), p)?;
            (format!("{hz} Hz"), polling_request(target.polling, p, hz)?)
        }
        Setting::SleepTime => {
            let v = query(s, &power::get_sleep_time(), power::parse_sleep_time)?;
            (format!("{v} s"), power::set_sleep_time(v))
        }
        Setting::Button { mode, button } => {
            // Rebuilt with the requested mode (the slot that was read); reply byte 2
            // is not the mode.
            let r = query(s, &buttons::get_button_assignment(p, button, mode), |a| {
                buttons::parse_button_assignment(a, mode)
            })?;
            (
                r.to_string(),
                buttons::set_button_assignment(&r.assignment)?,
            )
        }
    })
}

/// Result of [`apply`]: the lines to show and the profile that undoes the writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyReport {
    /// Settings written (or, in a dry run, to be written).
    pub writes: Vec<Setting>,
    /// Settings skipped because they already have the profile's value.
    pub unchanged: Vec<Setting>,
    /// Old values of `writes`; `None` when nothing is (to be) written. Present in a
    /// dry run too, so the caller can say where it would be saved.
    pub restore: Option<Profile>,
    pub lines: Vec<String>,
    /// A write failed part-way (earlier writes were sent; `restore` undoes them).
    pub failed: Option<String>,
}

impl ApplyReport {
    pub fn lines(&self) -> Vec<String> {
        self.lines.clone()
    }
}

/// Plan, check and (unless `dry_run`) write `profile` in one call, for callers that
/// cannot save the restore profile in between (the agent: the client saves it from
/// the report). Errors before the first write are `Err`; a failed write is a report
/// with `failed` set. `conflict` is running Razer software: a real apply is refused
/// before touching the device unless `opts.force` (a dry run only reads).
pub fn apply<D: FeatureIo>(
    s: &mut Session<D>,
    pid: u16,
    device: Option<&Device>,
    conflict: Option<&str>,
    profile: Profile,
    opts: ApplyOptions,
    dry_run: bool,
) -> Result<ApplyReport, ApplyError> {
    if let Some(name) = conflict
        && !dry_run
        && !opts.force
    {
        return Err(ApplyError::Conflict(name.to_owned()));
    }
    let plan = plan_apply(s, pid, device, profile, opts)?;
    let mut r = ApplyReport {
        writes: plan.writes.iter().map(|w| w.setting).collect(),
        unchanged: plan.unchanged.iter().map(|w| w.setting).collect(),
        restore: plan.restore().cloned(),
        lines: Vec::new(),
        failed: None,
    };
    if plan.sets_nothing() {
        r.lines.push("The profile sets nothing; no writes.".into());
        return Ok(r);
    }
    r.lines.extend(plan.unchanged_lines());
    let Some(snap) = &plan.snapshot else {
        r.lines.push(format!(
            "Nothing to change: the device already has all {} setting(s) of the profile; no writes.",
            plan.unchanged.len()
        ));
        return Ok(r);
    };
    r.lines.push(plan.summary_line());
    r.lines
        .extend(snap.notes.iter().map(|n| format!("note: {n}")));
    if dry_run {
        r.lines.extend(plan.dry_run_lines());
        r.lines.push("Run again with --apply to write.".into());
        return Ok(r);
    }
    plan.check_snapshot()?;
    match execute(s, &plan, |l| r.lines.push(l)) {
        Ok(()) => r.lines.push(format!(
            "Applied {} write(s); {} unchanged.",
            plan.writes.len(),
            plan.unchanged.len()
        )),
        Err(e) => {
            r.lines.extend(e.stopped_line());
            r.failed = Some(e.to_string());
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
