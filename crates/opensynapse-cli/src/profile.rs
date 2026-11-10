//! `profile export` / `profile apply`: device state <-> `opensynapse_core::Profile`.
//!
//! I/O stays in [`export`], [`apply`] and [`read_state`]; turning a read
//! [`DeviceState`] into a profile, the snapshot checks and the restore file name are
//! pure functions. Requests come only from `opensynapse-protocol` builders (directly
//! or through `Profile::plan`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use opensynapse_core::validate::{MAX_MOUSE_BUTTON, POLLING_RATES};
use opensynapse_core::{
    ButtonAction, ButtonId, DeviceLimits, DeviceSection, DpiSection, HyperShiftSection, Pid,
    PlannedWrite, PollingSection, PowerSection, Profile, Setting,
};
use opensynapse_device_registry::Device;
use opensynapse_protocol::buttons::{self, ButtonAssignment, Known, Mode};
use opensynapse_protocol::{Request, dpi, hid_usage, power};

use crate::{
    PollingVariant, Target, WriteOpts, active_label, polling_get, polling_request, read_button,
};

/// Onboard profile slot that `profile export` reads and `profile apply` writes
/// (button assignments; also the HyperPolling profile byte).
pub(crate) const ONBOARD_PROFILE: u8 = 1;

/// Button IDs from `--buttons` (`1-5`, `1,2,4`, `1-3,6`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonList(pub Vec<u8>);

pub(crate) fn parse_button_list(s: &str) -> Result<ButtonList, String> {
    let num = |t: &str| -> Result<u8, String> {
        t.trim()
            .parse::<u8>()
            .ok()
            .filter(|&b| b >= 1)
            .ok_or_else(|| format!("`{}` is not a button ID (1-255)", t.trim()))
    };
    let mut out = Vec::new();
    for part in s.split(',') {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (num(a)?, num(b)?),
            None => {
                let n = num(part)?;
                (n, n)
            }
        };
        if a > b {
            return Err(format!("range `{}` runs backwards", part.trim()));
        }
        for id in a..=b {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    Ok(ButtonList(out))
}

/// Raw device values, as read. `None` / missing entries were not read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeviceState {
    pub pid: u16,
    pub stages: Option<dpi::DpiStages>,
    pub polling_hz: Option<u32>,
    pub sleep: Option<u16>,
    pub buttons: Vec<ButtonAssignment>,
}

/// A profile built from a [`DeviceState`] plus notes about values it had to leave out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Export {
    pub profile: Profile,
    pub notes: Vec<String>,
}

/// Express `state` as a profile. Values the profile model cannot hold are left out
/// and listed (raw) in `notes`.
pub(crate) fn export_profile(state: &DeviceState) -> Export {
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
/// lists only HyperShift mappings that differ from the normal one. Only for
/// `profile export`: the restore snapshot keeps every slot it read, because it must
/// undo exactly what apply overwrites.
pub(crate) fn drop_redundant_hypershift(p: &mut Profile) {
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
        Known::MouseButton(_) | Known::DoubleClick(_) => return None,
    })
}

/// Profile text with a comment header.
pub(crate) fn render(profile: &Profile, header: &str) -> Result<String> {
    let mut out = String::new();
    for line in header.lines() {
        let _ = writeln!(out, "# {line}");
    }
    out.push_str(&profile.to_toml()?);
    Ok(out)
}

/// Device limits from the registry entry; unknown values fall back to "anything the
/// protocol allows". The registry has no stage-count limit yet, so that one always
/// falls back to the protocol maximum.
pub(crate) fn limits_for(device: Option<&Device>) -> DeviceLimits {
    let mut l = DeviceLimits::default();
    if let Some(d) = device {
        let u = |v: Option<u32>| v.and_then(|v| u16::try_from(v).ok());
        l.dpi_min = u(d.dpi_min).unwrap_or(l.dpi_min);
        l.dpi_max = u(d.dpi_max).unwrap_or(l.dpi_max);
        l.dpi_step = u(d.dpi_step).unwrap_or(l.dpi_step);
    }
    l
}

/// Where the snapshot for `input` goes: `<stem>.restore.toml` next to it, or
/// `<stem>.restore-2.toml`, `-3`, … if that exists (an earlier snapshot is never
/// overwritten).
pub(crate) fn restore_path(input: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let is_toml = input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"));
    let stem = if is_toml {
        input.file_stem()
    } else {
        input.file_name()
    };
    let stem = stem.map_or("profile".into(), |s| s.to_string_lossy());
    let dir = input.parent().unwrap_or(Path::new(""));
    (1u32..)
        .map(|n| {
            dir.join(if n == 1 {
                format!("{stem}.restore.toml")
            } else {
                format!("{stem}.restore-{n}.toml")
            })
        })
        .find(|p| !exists(p))
        .expect("some free name")
}

/// Why the snapshot cannot undo `plan`: a setting the plan overwrites is missing from
/// it (its old value could not be expressed), or it would not pass validation.
pub(crate) fn snapshot_problems(
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
pub(crate) fn setting_value(p: &Profile, setting: Setting) -> Option<String> {
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
    }
}

/// A plan split into the writes to send and the ones whose setting already has the
/// target value, plus the snapshot that undoes `writes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Prepared {
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
pub(crate) fn prepare(
    plan: Vec<PlannedWrite>,
    state: &DeviceState,
    target: &opensynapse_core::Target,
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

/// The request that would write the value `state` holds for `setting`, if it was
/// read and can be encoded. Same rebuild as [`read_back`].
fn current_request(
    state: &DeviceState,
    setting: Setting,
    target: &opensynapse_core::Target,
) -> Option<Request> {
    match setting {
        Setting::DpiStages => dpi::set_dpi_stages(target.dpi_storage, state.stages.as_ref()?).ok(),
        Setting::PollingRate => polling_request(
            PollingVariant::from(target.polling),
            target.onboard_profile,
            state.polling_hz?,
        )
        .ok(),
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
pub(crate) struct ReadSpec {
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

/// Read what `spec` lists. `lenient`: a failed read becomes a note instead of an
/// error. Also returns the polling variant the device answered (if polling was read).
fn read_state(
    dev: &mut Target,
    spec: &ReadSpec,
    lenient: bool,
) -> Result<(DeviceState, Option<PollingVariant>, Vec<String>)> {
    let mut notes = Vec::new();
    let mut state = DeviceState {
        pid: dev.entry.pid,
        ..DeviceState::default()
    };
    if let Some(storage) = spec.dpi {
        let r = dev.query(&dpi::get_dpi_stages(storage), dpi::parse_dpi_stages);
        state.stages = soft(&mut notes, lenient, "DPI stages".into(), r)?;
    }
    let mut variant = None;
    if spec.polling {
        // Same auto detection as `polling get|set`: period, then HyperPolling.
        let r = polling_get(dev, None, ONBOARD_PROFILE);
        if let Some((hz, v)) = soft(&mut notes, lenient, "polling rate".into(), r)? {
            state.polling_hz = Some(hz);
            variant = Some(v);
        }
    }
    if spec.sleep {
        let r = dev.query(&power::get_sleep_time(), power::parse_sleep_time);
        state.sleep = soft(&mut notes, lenient, "sleep time".into(), r)?;
    }
    for &(mode, b) in &spec.buttons {
        let r = read_button(dev, ONBOARD_PROFILE, b, mode);
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
    r: Result<T>,
) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if lenient => {
            notes.push(format!("{what} not read: {e}"));
            Ok(None)
        }
        Err(e) => Err(e.context(format!("cannot read {what}"))),
    }
}

fn header(dev: &Target, what: &str) -> String {
    format!(
        "OpenSynapse profile: {what} {} ({:04X}), onboard profile {ONBOARD_PROFILE}.",
        dev.name(),
        dev.entry.pid
    )
}

/// `profile export`.
pub(crate) fn export(dev: &mut Target, ids: &ButtonList, out: Option<&Path>) -> Result<()> {
    let (state, _, mut notes) = read_state(dev, &ReadSpec::export(&ids.0), true)?;
    let mut ex = export_profile(&state);
    drop_redundant_hypershift(&mut ex.profile);
    notes.extend(ex.notes);
    for n in &notes {
        eprintln!("note: {n}");
    }
    if let Err(e) = ex.profile.validate(&limits_for(dev.device)) {
        eprintln!("warning: the exported profile does not pass validation: {e}");
    }
    let text = render(&ex.profile, &header(dev, "exported from"))?;
    match out {
        Some(path) => {
            std::fs::write(path, &text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!("Wrote {}.", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

/// `profile apply`.
pub(crate) fn apply(
    dev: &mut Target,
    file: &Path,
    write: WriteOpts,
    volatile: bool,
    all: bool,
) -> Result<()> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let profile = Profile::from_toml(&text).with_context(|| file.display().to_string())?;
    let limits = limits_for(dev.device);
    if let Err(e) = profile.validate(&limits) {
        bail!("{}: {e}", file.display());
    }
    if let Some(d) = &profile.device
        && d.pid.0 != dev.entry.pid
        && !write.force
    {
        bail!(
            "{} is for PID {}, but the device is {:04X}; use --force to apply anyway",
            file.display(),
            d.pid,
            dev.entry.pid
        );
    }
    if let (Some(p), Some(d)) = (&profile.polling, dev.device)
        && !d.polling_rates.is_empty()
        && !d.polling_rates.contains(&p.hz)
        && !write.force
    {
        bail!(
            "{} Hz is not offered for this device ({:?}); use --force to override",
            p.hz,
            d.polling_rates
        );
    }

    let storage = if volatile {
        dpi::Storage::Volatile
    } else {
        dpi::Storage::Persistent
    };
    // Snapshot of everything the profile overwrites (also detects the polling variant).
    let (state, variant, _) = read_state(dev, &ReadSpec::for_profile(&profile, storage), false)?;
    let target = opensynapse_core::Target {
        limits,
        onboard_profile: ONBOARD_PROFILE,
        dpi_storage: storage,
        polling: variant.map(Into::into).unwrap_or_default(),
    };
    let plan = profile.plan(&target)?;
    if plan.is_empty() {
        println!("{} sets nothing; no writes.", file.display());
        return Ok(());
    }
    let Prepared {
        writes: plan,
        unchanged,
        snapshot,
    } = prepare(plan, &state, &target, all);
    for w in &unchanged {
        let v = setting_value(&profile, w.setting).unwrap_or_else(|| "?".into());
        println!("unchanged: {} ({v})", w.setting);
    }
    let Some(snap) = snapshot else {
        println!(
            "Nothing to change: the device already has all {} setting(s) of {}; no writes, no restore file.",
            unchanged.len(),
            file.display()
        );
        return Ok(());
    };
    println!("{} write(s), {} unchanged.", plan.len(), unchanged.len());
    for n in &snap.notes {
        eprintln!("note: {n}");
    }
    let problems = snapshot_problems(&plan, &snap.profile, &limits);
    let restore = restore_path(file, Path::exists);
    let hint = format!(
        "To restore: opensynapse --pid {:04X} profile apply {} --apply{}",
        dev.entry.pid,
        restore.display(),
        if volatile { " --volatile" } else { "" },
    );
    let change = |w: &PlannedWrite| {
        let old = setting_value(&snap.profile, w.setting).unwrap_or_else(|| "?".into());
        let new = setting_value(&profile, w.setting).unwrap_or_else(|| "?".into());
        format!("{}: {old} -> {new}", w.setting)
    };

    if !write.apply {
        for w in &plan {
            println!("Dry run: would change {}.", change(w));
            println!("Report: {:?}", w.request.encode(0));
        }
        for p in &problems {
            println!("Refusal (--apply needs --force): {p}");
        }
        println!(
            "With --apply, the current values are saved to {} first.",
            restore.display()
        );
        println!("Run again with --apply to write.");
        return Ok(());
    }
    if !problems.is_empty() && !write.force {
        bail!(
            "the snapshot taken before writing could not undo this profile:\n  {}\n\
             Nothing was written. Use --force to apply anyway.",
            problems.join("\n  ")
        );
    }
    dev.guard(write)?;
    let snap_text = render(&snap.profile, &header(dev, "values before applying to"))?;
    std::fs::write(&restore, snap_text)
        .with_context(|| format!("cannot write the restore file {}", restore.display()))?;
    println!("Saved the current values to {}.", restore.display());
    println!("{hint}");
    for (i, w) in plan.iter().enumerate() {
        if let Err(e) = write_one(dev, w, &target) {
            println!(
                "Stopped at write {} of {} ({}); later writes were not sent.",
                i + 1,
                plan.len(),
                w.setting
            );
            println!("{hint}");
            return Err(e);
        }
        println!("Changed {}.", change(w));
    }
    println!(
        "Applied {} write(s) from {}; {} unchanged.",
        plan.len(),
        file.display(),
        unchanged.len()
    );
    Ok(())
}

/// Send one planned write, read the setting back and check that re-encoding the
/// read-back value gives the same request.
fn write_one(dev: &mut Target, w: &PlannedWrite, target: &opensynapse_core::Target) -> Result<()> {
    dev.session.execute(&w.request)?;
    let (now, rebuilt) = read_back(dev, w.setting, target)?;
    if rebuilt != w.request {
        bail!("{} reads back as {now}, not the value written", w.setting);
    }
    println!("Verified: {} is now {now}.", w.setting);
    Ok(())
}

/// The current value of `setting` (for display) and the request that would write it.
fn read_back(
    dev: &mut Target,
    setting: Setting,
    target: &opensynapse_core::Target,
) -> Result<(String, Request)> {
    let p = target.onboard_profile;
    Ok(match setting {
        Setting::DpiStages => {
            let s = dev.query(
                &dpi::get_dpi_stages(target.dpi_storage),
                dpi::parse_dpi_stages,
            )?;
            let list: Vec<String> = s.stages.iter().map(|x| x.dpi.to_string()).collect();
            let label = format!("[{}] (active {})", list.join(", "), active_label(&s));
            (label, dpi::set_dpi_stages(target.dpi_storage, &s)?)
        }
        Setting::PollingRate => {
            let v = PollingVariant::from(target.polling);
            let (hz, _) = polling_get(dev, Some(v), p)?;
            (format!("{hz} Hz"), polling_request(v, p, hz)?)
        }
        Setting::SleepTime => {
            let v = dev.query(&power::get_sleep_time(), power::parse_sleep_time)?;
            (format!("{v} s"), power::set_sleep_time(v))
        }
        Setting::Button { mode, button } => {
            // Rebuilt with the requested mode (the slot that was read); reply byte 2
            // is not the mode.
            let r = read_button(dev, p, button, mode)?;
            (
                r.to_string(),
                buttons::set_button_assignment(&r.assignment)?,
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensynapse_core::Target as PlanTarget;
    use opensynapse_protocol::dpi::{Dpi, DpiStage, DpiStages};

    fn stages(active: u8, dpis: &[u16]) -> DpiStages {
        DpiStages {
            active,
            stages: dpis
                .iter()
                .zip(1u8..)
                .map(|(&v, id)| DpiStage {
                    id,
                    dpi: Dpi::uniform(v),
                })
                .collect(),
        }
    }

    fn raw(button: u8, mode: Mode, function: u8, data: Vec<u8>) -> ButtonAssignment {
        ButtonAssignment {
            profile: 1,
            button,
            mode,
            function,
            data,
        }
    }

    fn viper() -> DeviceLimits {
        DeviceLimits {
            dpi_min: 100,
            dpi_max: 30000,
            dpi_step: 50,
            ..DeviceLimits::default()
        }
    }

    fn fake_state() -> DeviceState {
        DeviceState {
            pid: 0x00B8,
            stages: Some(stages(2, &[400, 800, 1600])),
            polling_hz: Some(1000),
            sleep: Some(300),
            buttons: vec![
                buttons::mouse_button(1, 1, Mode::Normal, 1),
                buttons::keyboard(1, 4, Mode::Normal, 0x05, 0x04),
                raw(5, Mode::Normal, 9, vec![1, 2]),
                buttons::dpi_up(1, 4, Mode::HyperShift),
                buttons::double_click(1, 5, Mode::HyperShift, 1),
            ],
        }
    }

    #[test]
    fn export_formats_toml() {
        let ex = export_profile(&fake_state());
        let text = render(&ex.profile, "header line").unwrap();
        assert_eq!(
            text,
            r#"# header line
[device]
pid = "00B8"

[dpi]
stages = [400, 800, 1600]
active_stage = 2

[polling]
hz = 1000

[power]
sleep_seconds = 300

[buttons.1]
action = "mouse-button"
target = 1

[buttons.4]
action = "key"
key = "a"
mods = ["ctrl", "alt"]

[hypershift.buttons.4]
action = "dpi-up"

[hypershift.buttons.5]
action = "double-click"
target = 1
"#
        );
        assert_eq!(
            ex.notes,
            ["button 5 (normal mode) left out: function 9, data [1, 2] cannot be expressed"]
        );
        // Parses back and validates.
        let back = Profile::from_toml(&text).unwrap();
        assert_eq!(back, ex.profile);
        assert_eq!(back.validate(&viper()), Ok(()));
    }

    #[test]
    fn export_leaves_out_unexpressible() {
        let mut odd = stages(2, &[400, 800]);
        odd.stages[1].dpi = Dpi { x: 800, y: 400 };
        let state = DeviceState {
            pid: 0x00B8,
            stages: Some(odd),
            polling_hz: Some(8000),
            sleep: None,
            buttons: vec![
                buttons::keyboard(1, 4, Mode::Normal, 0x00, 0x50), // unnamed usage
                buttons::keyboard(1, 5, Mode::Normal, 0x10, 0x04), // right ctrl
                raw(6, Mode::Normal, 6, vec![2]),                  // DPI, not "up"
                buttons::mouse_button(1, 7, Mode::Normal, 9),      // target out of range
            ],
        };
        let ex = export_profile(&state);
        assert_eq!(ex.profile.dpi, None);
        assert_eq!(ex.profile.polling, None);
        assert_eq!(ex.profile.power, None);
        assert!(ex.profile.buttons.is_empty() && ex.profile.hypershift.is_none());
        assert_eq!(ex.notes.len(), 6, "{:#?}", ex.notes);
        assert!(ex.notes[0].contains("different X/Y DPI 800x400"));
        assert!(ex.notes[1].contains("8000 Hz"));
        // Stage IDs that are not positions and an active ID that matches nothing.
        let mut ids = stages(1, &[400, 800]);
        ids.stages[0].id = 3;
        assert!(dpi_section(&ids).unwrap_err().contains("has ID 3"));
        assert!(
            dpi_section(&stages(9, &[400]))
                .unwrap_err()
                .contains("ID 9")
        );
    }

    #[test]
    fn button_list_parsing() {
        assert_eq!(
            parse_button_list("1-5"),
            Ok(ButtonList(vec![1, 2, 3, 4, 5]))
        );
        assert_eq!(parse_button_list("4"), Ok(ButtonList(vec![4])));
        assert_eq!(
            parse_button_list("1-3, 6,2"),
            Ok(ButtonList(vec![1, 2, 3, 6]))
        );
        for bad in ["", "0", "5-1", "x", "1-", "256"] {
            assert!(parse_button_list(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn restore_file_name() {
        let none = |_: &Path| false;
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            restore_path(&p("dir/gaming.toml"), none),
            p("dir/gaming.restore.toml")
        );
        assert_eq!(
            restore_path(&p("gaming.TOML"), none),
            p("gaming.restore.toml")
        );
        assert_eq!(restore_path(&p("gaming"), none), p("gaming.restore.toml"));
        assert_eq!(restore_path(&p("a.b.txt"), none), p("a.b.txt.restore.toml"));
        // Applying a restore file snapshots to a new file, not over itself.
        assert_eq!(
            restore_path(&p("gaming.restore.toml"), none),
            p("gaming.restore.restore.toml")
        );
        // Earlier snapshots are kept.
        let taken = [p("g.restore.toml"), p("g.restore-2.toml")];
        assert_eq!(
            restore_path(&p("g.toml"), |x| taken.iter().any(|t| t == x)),
            p("g.restore-3.toml")
        );
    }

    fn plan_for(text: &str) -> Vec<PlannedWrite> {
        let target = PlanTarget {
            limits: viper(),
            ..PlanTarget::default()
        };
        Profile::from_toml(text).unwrap().plan(&target).unwrap()
    }

    const APPLY: &str = r#"
[dpi]
stages = [800, 1600]
active_stage = 1
[polling]
hz = 500
[buttons.4]
action = "dpi-up"
[hypershift.buttons.5]
action = "mouse-button"
target = 4
"#;

    #[test]
    fn complete_snapshot_is_accepted() {
        let state = DeviceState {
            pid: 0x00B8,
            stages: Some(stages(1, &[400, 800, 1600])),
            polling_hz: Some(1000),
            sleep: None,
            buttons: vec![
                buttons::mouse_button(1, 4, Mode::Normal, 4),
                buttons::mouse_button(1, 5, Mode::HyperShift, 5),
            ],
        };
        let snap = export_profile(&state).profile;
        assert_eq!(
            snapshot_problems(&plan_for(APPLY), &snap, &viper()),
            [""; 0]
        );
        // The snapshot plans the same settings back.
        let undo: Vec<Setting> = plan_for(&snap.to_toml().unwrap())
            .iter()
            .map(|w| w.setting)
            .collect();
        let todo: Vec<Setting> = plan_for(APPLY).iter().map(|w| w.setting).collect();
        assert_eq!(undo, todo);
    }

    #[test]
    fn refuses_when_snapshot_loses_information() {
        let state = DeviceState {
            pid: 0x00B8,
            stages: Some(stages(1, &[400, 800])),
            polling_hz: Some(8000),
            sleep: None,
            buttons: vec![
                raw(4, Mode::Normal, 9, vec![1]),
                buttons::mouse_button(1, 5, Mode::HyperShift, 5),
            ],
        };
        let snap = export_profile(&state).profile;
        let problems = snapshot_problems(&plan_for(APPLY), &snap, &viper());
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(problems[0].starts_with("polling rate:"));
        assert!(problems[1].starts_with("button 4 (normal mode):"));

        // A snapshot that would not validate (DPI off the device's step).
        let state = DeviceState {
            pid: 0x00B8,
            stages: Some(stages(1, &[425])),
            ..DeviceState::default()
        };
        let snap = export_profile(&state).profile;
        let problems = snapshot_problems(
            &plan_for("[dpi]\nstages=[800]\nactive_stage=1"),
            &snap,
            &viper(),
        );
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].contains("would not pass validation"),
            "{problems:?}"
        );
    }

    #[test]
    fn read_spec_covers_what_apply_overwrites() {
        let p = Profile::from_toml(APPLY).unwrap();
        let spec = ReadSpec::for_profile(&p, dpi::Storage::Volatile);
        assert_eq!(spec.dpi, Some(dpi::Storage::Volatile));
        assert!(spec.polling && !spec.sleep);
        assert_eq!(spec.buttons, [(Mode::Normal, 4), (Mode::HyperShift, 5)]);
        let e = ReadSpec::export(&[1, 2]);
        assert_eq!(
            e.buttons,
            [
                (Mode::Normal, 1),
                (Mode::Normal, 2),
                (Mode::HyperShift, 1),
                (Mode::HyperShift, 2)
            ]
        );
    }

    #[test]
    fn export_keeps_only_differing_hypershift() {
        let state = DeviceState {
            pid: 0x00B8,
            buttons: vec![
                buttons::mouse_button(1, 4, Mode::Normal, 4),
                buttons::mouse_button(1, 5, Mode::Normal, 5),
                raw(6, Mode::Normal, 9, vec![1]),
                buttons::mouse_button(1, 4, Mode::HyperShift, 4), // same as normal
                buttons::mouse_button(1, 5, Mode::HyperShift, 4), // differs
                buttons::dpi_up(1, 6, Mode::HyperShift),          // normal not expressible
            ],
            ..DeviceState::default()
        };
        let mut p = export_profile(&state).profile;
        // The snapshot form keeps every slot read.
        assert_eq!(p.hypershift.as_ref().unwrap().buttons.len(), 3);
        drop_redundant_hypershift(&mut p);
        let hs: Vec<u8> = p
            .hypershift
            .as_ref()
            .unwrap()
            .buttons
            .keys()
            .map(|b| b.0)
            .collect();
        assert_eq!(hs, [5, 6]);
        assert_eq!(
            setting_value(
                &p,
                Setting::Button {
                    mode: Mode::HyperShift,
                    button: 5
                }
            )
            .as_deref(),
            Some("mouse button 4")
        );
        // All equal: no [hypershift] section at all.
        let mut same = export_profile(&DeviceState {
            buttons: vec![
                buttons::mouse_button(1, 4, Mode::Normal, 4),
                buttons::mouse_button(1, 4, Mode::HyperShift, 4),
            ],
            ..DeviceState::default()
        })
        .profile;
        drop_redundant_hypershift(&mut same);
        assert!(same.hypershift.is_none());
        assert!(!same.to_toml().unwrap().contains("hypershift"));
    }

    /// Apply snapshots the HyperShift slot it overwrites, even when it equals the
    /// normal mapping (the snapshot is not deduplicated).
    #[test]
    fn snapshot_keeps_hypershift_equal_to_normal() {
        let p = Profile::from_toml(APPLY).unwrap();
        let spec = ReadSpec::for_profile(&p, dpi::Storage::Persistent);
        assert!(spec.buttons.contains(&(Mode::HyperShift, 5)));
        let state = DeviceState {
            pid: 0x00B8,
            stages: Some(stages(1, &[400, 800])),
            polling_hz: Some(1000),
            sleep: None,
            buttons: vec![
                buttons::mouse_button(1, 4, Mode::Normal, 5),
                buttons::mouse_button(1, 5, Mode::Normal, 5),
                buttons::mouse_button(1, 5, Mode::HyperShift, 5),
            ],
        };
        let snap = export_profile(&state).profile;
        assert_eq!(
            snap.buttons.get(&ButtonId(5)),
            Some(&ButtonAction::MouseButton { target: 5 })
        );
        assert_eq!(
            setting_value(
                &snap,
                Setting::Button {
                    mode: Mode::HyperShift,
                    button: 5
                }
            )
            .as_deref(),
            Some("mouse button 5")
        );
        assert!(snapshot_problems(&plan_for(APPLY), &snap, &viper()).is_empty());
    }

    /// Device state equal to everything `APPLY` sets.
    fn state_matching_apply() -> DeviceState {
        DeviceState {
            pid: 0x00B8,
            stages: Some(stages(1, &[800, 1600])),
            polling_hz: Some(500),
            sleep: Some(300),
            buttons: vec![
                buttons::dpi_up(1, 4, Mode::Normal),
                buttons::mouse_button(1, 5, Mode::HyperShift, 4),
            ],
        }
    }

    fn settings(ws: &[PlannedWrite]) -> Vec<Setting> {
        ws.iter().map(|w| w.setting).collect()
    }

    #[test]
    fn all_unchanged_means_no_writes_and_no_restore_file() {
        let target = PlanTarget {
            limits: viper(),
            ..PlanTarget::default()
        };
        let p = prepare(plan_for(APPLY), &state_matching_apply(), &target, false);
        assert!(p.writes.is_empty());
        assert_eq!(p.unchanged.len(), 4);
        assert_eq!(p.snapshot, None);
    }

    #[test]
    fn one_change_means_one_write_and_a_minimal_snapshot() {
        let target = PlanTarget {
            limits: viper(),
            ..PlanTarget::default()
        };
        let mut state = state_matching_apply();
        state.buttons[1] = buttons::mouse_button(1, 5, Mode::HyperShift, 5);
        let p = prepare(plan_for(APPLY), &state, &target, false);
        let hs5 = Setting::Button {
            mode: Mode::HyperShift,
            button: 5,
        };
        assert_eq!(settings(&p.writes), [hs5]);
        assert_eq!(
            settings(&p.unchanged),
            [
                Setting::DpiStages,
                Setting::PollingRate,
                Setting::Button {
                    mode: Mode::Normal,
                    button: 4
                }
            ]
        );
        // The snapshot holds only the old value of what is written, and undoes it.
        let snap = p.snapshot.unwrap();
        let mut want = Profile {
            device: Some(DeviceSection { pid: Pid(0x00B8) }),
            ..Profile::default()
        };
        want.hypershift = Some(HyperShiftSection::default());
        want.hypershift
            .as_mut()
            .unwrap()
            .buttons
            .insert(ButtonId(5), ButtonAction::MouseButton { target: 5 });
        assert_eq!(snap.profile, want);
        assert!(snapshot_problems(&p.writes, &snap.profile, &viper()).is_empty());
        assert_eq!(settings(&plan_for(&snap.profile.to_toml().unwrap())), [hs5]);

        // A different active stage or another polling variant is a change too.
        let mut state = state_matching_apply();
        state.stages = Some(stages(2, &[800, 1600]));
        let p = prepare(plan_for(APPLY), &state, &target, false);
        assert_eq!(settings(&p.writes), [Setting::DpiStages]);
        let hyper = PlanTarget {
            polling: opensynapse_core::PollingVariant::Hyper,
            ..target
        };
        let p = prepare(plan_for(APPLY), &state_matching_apply(), &hyper, false);
        assert_eq!(settings(&p.writes), [Setting::PollingRate]);
        // A setting that was not read counts as changed.
        let mut state = state_matching_apply();
        state.buttons.clear();
        let p = prepare(plan_for(APPLY), &state, &target, false);
        assert_eq!(p.writes.len(), 2);
    }

    #[test]
    fn all_flag_writes_unchanged_values() {
        let target = PlanTarget {
            limits: viper(),
            ..PlanTarget::default()
        };
        let state = state_matching_apply();
        let p = prepare(plan_for(APPLY), &state, &target, true);
        assert_eq!(settings(&p.writes), settings(&plan_for(APPLY)));
        assert!(p.unchanged.is_empty());
        let snap = p.snapshot.unwrap().profile;
        assert!(snapshot_problems(&p.writes, &snap, &viper()).is_empty());
        // Sleep was read but is not in the profile, so it is not in the snapshot.
        assert_eq!(snap.power, None);
    }

    #[test]
    fn limits_from_registry() {
        let reg = opensynapse_device_registry::Registry::embedded();
        let viper = reg
            .lookup(opensynapse_device_registry::RAZER_VID, 0x00B8)
            .unwrap()
            .device();
        let l = limits_for(Some(viper));
        assert_eq!((l.dpi_min, l.dpi_max, l.dpi_step), (100, 30000, 50));
        assert_eq!(limits_for(None), DeviceLimits::default());
    }
}
