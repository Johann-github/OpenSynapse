//! Engine tests: pure (state -> profile, plan splitting, snapshot checks) and
//! against the fake mouse (`fake.rs`).

use std::time::Duration;

use super::*;
use crate::fake::FakeMouse;
use opensynapse_core::Target as PlanTarget;
use opensynapse_device_registry::{RAZER_VID, Registry};
use opensynapse_hid::Timing;
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
            raw(6, Mode::Normal, 6, vec![3]),                  // DPI, not up/down
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

fn plan_target() -> PlanTarget {
    PlanTarget {
        limits: viper(),
        ..PlanTarget::default()
    }
}

#[test]
fn all_unchanged_means_no_writes_and_no_restore_file() {
    let p = prepare(
        plan_for(APPLY),
        &state_matching_apply(),
        &plan_target(),
        false,
    );
    assert!(p.writes.is_empty());
    assert_eq!(p.unchanged.len(), 4);
    assert_eq!(p.snapshot, None);
}

#[test]
fn one_change_means_one_write_and_a_minimal_snapshot() {
    let target = plan_target();
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
        polling: PollingVariant::Hyper,
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
    let state = state_matching_apply();
    let p = prepare(plan_for(APPLY), &state, &plan_target(), true);
    assert_eq!(settings(&p.writes), settings(&plan_for(APPLY)));
    assert!(p.unchanged.is_empty());
    let snap = p.snapshot.unwrap().profile;
    assert!(snapshot_problems(&p.writes, &snap, &viper()).is_empty());
    // Sleep was read but is not in the profile, so it is not in the snapshot.
    assert_eq!(snap.power, None);
}

#[test]
fn limits_from_registry() {
    let l = limits_for(Some(viper_device()));
    assert_eq!((l.dpi_min, l.dpi_max, l.dpi_step), (100, 30000, 50));
    assert_eq!(limits_for(None), DeviceLimits::default());
}

// ---- against the fake mouse ----

const PID: u16 = 0x00B8;

fn viper_device() -> &'static Device {
    Registry::embedded()
        .lookup(RAZER_VID, PID)
        .expect("Viper V3 HyperSpeed in the registry")
        .device()
}

fn session(m: &FakeMouse) -> Session<FakeMouse> {
    Session::new(
        m.clone(),
        Timing {
            between_out: Duration::ZERO,
            out_to_in: Duration::ZERO,
            between_in: Duration::ZERO,
            max_in: 2,
            max_out: 2,
        },
    )
}

/// Everything the fake mouse holds by default (stages 400 [800], 1000 Hz, sleep
/// 300, button N -> mouse button N in both modes).
const FAKE_DEFAULTS: &str = r#"
[device]
pid = "00B8"
[dpi]
stages = [400, 800]
active_stage = 2
[polling]
hz = 1000
[power]
sleep_seconds = 300
[buttons.4]
action = "mouse-button"
target = 4
[hypershift.buttons.5]
action = "mouse-button"
target = 5
"#;

fn run(
    m: &FakeMouse,
    text: &str,
    conflict: Option<&str>,
    opts: ApplyOptions,
    dry_run: bool,
) -> Result<ApplyReport, ApplyError> {
    apply(
        &mut session(m),
        PID,
        Some(viper_device()),
        conflict,
        Profile::from_toml(text).unwrap(),
        opts,
        dry_run,
    )
}

fn real(m: &FakeMouse, text: &str) -> ApplyReport {
    run(m, text, None, ApplyOptions::default(), false).unwrap()
}

#[test]
fn fake_unchanged_means_zero_writes() {
    let m = FakeMouse::new();
    let r = real(&m, FAKE_DEFAULTS);
    assert_eq!(m.writes(), 0);
    assert!(r.writes.is_empty());
    assert_eq!(r.unchanged.len(), 5);
    assert_eq!(r.restore, None);
    assert_eq!(r.failed, None);
    assert_eq!(
        r.lines.last().unwrap(),
        "Nothing to change: the device already has all 5 setting(s) of the profile; no writes."
    );
    assert_eq!(r.lines[0], "unchanged: DPI stages ([400, 800] (active 2))");
}

#[test]
fn fake_one_change_writes_once_and_restores() {
    let m = FakeMouse::new();
    let text = FAKE_DEFAULTS.replace("hz = 1000", "hz = 500");
    let r = real(&m, &text);
    assert_eq!(m.writes(), 1);
    assert_eq!(m.state().polling_hz, 500);
    assert_eq!(r.writes, [Setting::PollingRate]);
    assert_eq!(r.failed, None);
    assert!(
        r.lines
            .contains(&"Verified: polling rate is now 500 Hz.".to_string())
    );
    assert!(
        r.lines
            .contains(&"Changed polling rate: 1000 Hz -> 500 Hz.".to_string())
    );
    assert_eq!(r.lines.last().unwrap(), "Applied 1 write(s); 4 unchanged.");
    // The restore profile holds exactly the old polling rate.
    let restore = r.restore.unwrap();
    assert_eq!(
        restore,
        Profile {
            device: Some(DeviceSection { pid: Pid(PID) }),
            polling: Some(PollingSection { hz: 1000 }),
            ..Profile::default()
        }
    );
    // Applying it undoes the change with one write.
    let back = real(&m, &restore.to_toml().unwrap());
    assert_eq!(back.writes, [Setting::PollingRate]);
    assert_eq!(m.state().polling_hz, 1000);
    assert_eq!(m.writes(), 2);
}

#[test]
fn fake_all_writes_everything() {
    let m = FakeMouse::new();
    let r = run(
        &m,
        FAKE_DEFAULTS,
        None,
        ApplyOptions {
            all: true,
            ..ApplyOptions::default()
        },
        false,
    )
    .unwrap();
    assert_eq!(m.writes(), 5);
    assert_eq!(r.writes.len(), 5);
    assert!(r.unchanged.is_empty());
    // The restore profile equals the profile (the device had those values).
    assert_eq!(r.restore, Some(Profile::from_toml(FAKE_DEFAULTS).unwrap()));
}

#[test]
fn fake_dry_run_reads_only() {
    let m = FakeMouse::new();
    let text = FAKE_DEFAULTS.replace("target = 4", "target = 5");
    // A dry run is allowed while Razer software runs.
    let r = run(
        &m,
        &text,
        Some("RazerAppEngine.exe"),
        ApplyOptions::default(),
        true,
    )
    .unwrap();
    assert_eq!(m.writes(), 0);
    assert!(r.restore.is_some(), "the caller says where it would go");
    assert!(
        r.lines.contains(
            &"Dry run: would change button 4 (normal mode): mouse button 4 -> mouse button 5."
                .to_string()
        ),
        "{:#?}",
        r.lines
    );
    assert_eq!(r.lines.last().unwrap(), "Run again with --apply to write.");
}

#[test]
fn fake_conflict_refused_unless_forced() {
    let m = FakeMouse::new();
    let text = FAKE_DEFAULTS.replace("hz = 1000", "hz = 500");
    let e = run(
        &m,
        &text,
        Some("RazerAppEngine.exe"),
        ApplyOptions::default(),
        false,
    )
    .unwrap_err();
    assert!(matches!(e, ApplyError::Conflict(ref n) if n == "RazerAppEngine.exe"));
    assert!(m.sent().is_empty(), "refused before touching the device");
    let forced = ApplyOptions {
        force: true,
        ..ApplyOptions::default()
    };
    run(&m, &text, Some("RazerAppEngine.exe"), forced, false).unwrap();
    assert_eq!(m.state().polling_hz, 500);
}

#[test]
fn fake_wrong_pid_and_invalid_profile() {
    let m = FakeMouse::new();
    let text = FAKE_DEFAULTS.replace("00B8", "00B9");
    let e = real_err(&m, &text);
    assert_eq!(
        e.to_string(),
        "the profile is for PID 00B9, but the device is 00B8; use --force to apply anyway"
    );
    let e = real_err(&m, &FAKE_DEFAULTS.replace("400, 800", "425, 800"));
    assert!(matches!(e, ApplyError::Invalid(_)), "{e}");
    assert!(m.sent().is_empty());
}

fn real_err(m: &FakeMouse, text: &str) -> ApplyError {
    run(m, text, None, ApplyOptions::default(), false).unwrap_err()
}

#[test]
fn fake_read_back_mismatch_is_a_failed_report() {
    let m = FakeMouse::new();
    m.state().ignore_writes = true;
    let text = FAKE_DEFAULTS.replace("sleep_seconds = 300", "sleep_seconds = 600");
    let r = real(&m, &text);
    assert_eq!(
        r.failed.as_deref(),
        Some("sleep time reads back as 300 s, not the value written")
    );
    assert_eq!(
        r.lines.last().unwrap(),
        "Stopped at write 1 of 1 (sleep time); later writes were not sent."
    );
    assert!(r.restore.is_some());
}

#[test]
fn fake_snapshot_problem_refuses_unless_forced() {
    let m = FakeMouse::new();
    // Button 4 holds something a profile cannot express.
    m.state().slots.insert((0, 4), (9, vec![1]));
    let text = FAKE_DEFAULTS.replace("target = 4", "target = 5");
    let e = real_err(&m, &text);
    assert!(matches!(e, ApplyError::SnapshotIncomplete(ref p) if p.len() == 1));
    assert_eq!(m.writes(), 0);
    let forced = ApplyOptions {
        force: true,
        ..ApplyOptions::default()
    };
    let r = run(&m, &text, None, forced, false).unwrap();
    assert_eq!(r.failed, None);
    assert_eq!(m.writes(), 1);
}

#[test]
fn fake_export_reads_everything() {
    let m = FakeMouse::new();
    let ex = export(&mut session(&m), PID, &[1, 2, 3, 4, 5]).unwrap();
    assert!(ex.notes.is_empty(), "{:?}", ex.notes);
    assert_eq!(ex.profile.buttons.len(), 5);
    // HyperShift equals normal on the fake: dropped.
    assert_eq!(ex.profile.hypershift, None);
    assert_eq!(ex.profile.polling, Some(PollingSection { hz: 1000 }));
    // A failed read becomes a note.
    m.state().answers = false;
    let ex = export(&mut session(&m), PID, &[1]).unwrap();
    assert_eq!(ex.notes.len(), 5, "{:?}", ex.notes);
    assert!(ex.notes[0].starts_with("DPI stages not read:"));
}
