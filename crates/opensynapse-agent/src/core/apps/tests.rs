//! Per-app switching against fake mice. Profile and rules files go to a fresh
//! temp folder per test; no real hardware.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use opensynapse_device::fake::{FakeMouse, default_stages};
use opensynapse_ipc::msg::{Event, Reply, Request};

use crate::core::{Core, CoreMsg};
use crate::events::NoEvents;
use crate::hardware::fake::{FakeHardware, VIPER_PID};

const A: &str = r"\\?\hid#vid_1532&pid_00b8&mi_00#a";

const CS2: &str = "[device]\npid = \"00B8\"\n[polling]\nhz = 500\n";
const DESK: &str = "[device]\npid = \"00B8\"\n[power]\nsleep_seconds = 120\n";
const STAGES: &str = "[device]\npid = \"00B8\"\n[dpi]\nstages = [1600, 3200]\nactive_stage = 2\n";
const BUTTONS: &str = "[device]\npid = \"00B8\"\n[buttons.4]\naction = \"dpi-down\"\n";
const OTHER_PID: &str = "[device]\npid = \"00C0\"\n[polling]\nhz = 500\n";
const NO_PID: &str = "[polling]\nhz = 500\n";

/// A fresh folder with `profiles/*.toml` and `app-profiles.toml`.
fn setup(test: &str, rules: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "opensynapse-agent-apps-{}-{test}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("profiles")).unwrap();
    for (name, text) in [
        ("cs2", CS2),
        ("desk", DESK),
        ("stages", STAGES),
        ("buttons", BUTTONS),
        ("other-pid", OTHER_PID),
        ("no-pid", NO_PID),
    ] {
        std::fs::write(dir.join("profiles").join(format!("{name}.toml")), text).unwrap();
    }
    let file = dir.join("app-profiles.toml");
    std::fs::write(&file, rules).unwrap();
    file
}

struct T {
    core: Core<FakeHardware>,
    mouse: FakeMouse,
    events: Receiver<Event>,
    file: PathBuf,
}

/// Core with one Viper, the rules loaded, and a subscriber.
fn start(test: &str, rules: &str) -> T {
    let (tx, _rx) = channel();
    let mut core = Core::new(
        FakeHardware::default(),
        Box::new(NoEvents),
        tx,
        Arc::new(AtomicBool::new(false)),
        Duration::ZERO,
    );
    let mouse = core.hw().plug(A, VIPER_PID);
    core.rescan();
    let file = setup(test, rules);
    core.set_app_profiles_file(Some(file.clone()));
    core.load_app_profiles();
    let (etx, events) = channel();
    core.dispatch(CoreMsg::Subscribe(etx));
    // Drop the DeviceArrived from the scan above (sent before subscribing: none).
    T {
        core,
        mouse,
        events,
        file,
    }
}

impl T {
    fn focus(&mut self, exe: &str) {
        assert!(!self.core.dispatch(CoreMsg::Foreground { exe: exe.into() }));
    }

    /// Every `AppProfile` event so far: (profile file name, error).
    fn switches(&self) -> Vec<(Option<String>, Option<String>, Vec<String>)> {
        self.events
            .try_iter()
            .filter_map(|e| match e {
                Event::AppProfile {
                    profile,
                    error,
                    lines,
                    ..
                } => Some((profile.map(|p| file_name(&p)), error, lines)),
                _ => None,
            })
            .collect()
    }

    fn info(&mut self) -> opensynapse_ipc::msg::AppProfilesInfo {
        match self.core.handle(Request::AppProfiles) {
            Ok(Reply::AppProfiles(i)) => i,
            other => panic!("expected AppProfiles, got {other:?}"),
        }
    }
}

/// (polling Hz, sleep s), read under one lock.
fn poll_sleep(m: &FakeMouse) -> (u32, u16) {
    let st = m.state();
    (st.polling_hz, st.sleep)
}

fn file_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

const RULES: &str = "[[app]]\nexe = \"cs2.exe\"\nprofile = \"profiles/cs2.toml\"\n\
                     [[app]]\nexe = \"code.exe\"\nprofile = \"profiles/desk.toml\"\n";

#[test]
fn match_applies_the_profile() {
    let mut t = start("match", RULES);
    t.focus(r"C:\Games\CS2.exe");
    assert_eq!(t.mouse.state().polling_hz, 500);
    assert_eq!(t.mouse.writes(), 1);
    let s = t.switches();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].0.as_deref(), Some("cs2.toml"));
    assert_eq!(s[0].1, None);
    assert!(
        s[0].2
            .contains(&"Changed polling rate: 1000 Hz -> 500 Hz.".to_string())
    );
    let i = t.info();
    assert!(i.enabled && i.restore_pending);
    assert_eq!(i.rules.len(), 2);
    assert_eq!(file_name(i.active.as_deref().unwrap()), "cs2.toml");
    assert_eq!(i.foreground.as_deref(), Some(r"C:\Games\CS2.exe"));
}

#[test]
fn same_profile_writes_nothing() {
    let rules = "[[app]]\nexe = \"a.exe\"\nprofile = \"profiles/cs2.toml\"\n\
                 [[app]]\nexe = \"b.exe\"\nprofile = \"profiles/cs2.toml\"\n";
    let mut t = start("same", rules);
    t.focus("a.exe");
    let sent = t.mouse.sent().len();
    t.focus("b.exe");
    t.focus("a.exe");
    assert_eq!(t.mouse.sent().len(), sent, "not even a read");
    assert_eq!(t.switches().len(), 1);
}

#[test]
fn unmatched_without_default_restores_and_forgets() {
    let mut t = start("restore", RULES);
    t.focus("cs2.exe");
    t.focus("code.exe");
    assert_eq!(
        poll_sleep(&t.mouse),
        (500, 120),
        "cs2's polling stays, desk adds sleep"
    );
    t.focus("explorer.exe");
    assert_eq!(
        poll_sleep(&t.mouse),
        (1000, 300),
        "both back to the values from before the first app profile"
    );
    let s = t.switches();
    assert_eq!(s.len(), 3);
    assert_eq!((s[2].0.as_deref(), s[2].1.as_deref()), (None, None));
    let i = t.info();
    assert_eq!((i.active, i.restore_pending), (None, false));

    // Unmatched → unmatched: nothing to do, no event, no traffic.
    let sent = t.mouse.sent().len();
    t.focus("notepad.exe");
    assert_eq!(t.mouse.sent().len(), sent);
    assert!(t.switches().is_empty());
}

#[test]
fn unmatched_with_default_applies_default() {
    let rules = format!("default = \"profiles/desk.toml\"\n{RULES}");
    let mut t = start("default", &rules);
    t.focus("explorer.exe");
    assert_eq!(t.mouse.state().sleep, 120);
    t.focus("cs2.exe");
    t.focus("notepad.exe");
    // Back on the default: its sleep was already set, so nothing is written for it.
    assert_eq!(t.mouse.state().sleep, 120);
    assert_eq!(
        t.mouse.state().polling_hz,
        500,
        "default sets no polling rate"
    );
    let s = t.switches();
    assert_eq!(s.len(), 3);
    assert_eq!(s[2].0.as_deref(), Some("desk.toml"));
}

#[test]
fn razer_running_writes_nothing_and_retries_later() {
    let mut t = start("conflict", RULES);
    t.core.hw().conflict = Some("RazerAppEngine.exe".into());
    t.focus("cs2.exe");
    assert!(t.mouse.sent().is_empty(), "not touched at all");
    let s = t.switches();
    assert_eq!(s.len(), 1);
    assert!(s[0].1.as_deref().unwrap().contains("RazerAppEngine.exe"));
    assert_eq!(t.info().active, None);

    t.core.hw().conflict = None;
    t.focus("cs2.exe");
    assert_eq!(t.mouse.state().polling_hz, 500);
}

#[test]
fn unknown_pid_is_skipped() {
    let rules = "[[app]]\nexe = \"x.exe\"\nprofile = \"profiles/other-pid.toml\"\n\
                 [[app]]\nexe = \"y.exe\"\nprofile = \"profiles/no-pid.toml\"\n";
    let mut t = start("pid", rules);
    t.focus("x.exe");
    assert!(t.mouse.sent().is_empty());
    let s = t.switches();
    assert_eq!(s[0].1, None);
    assert_eq!(
        s[0].2,
        vec!["No connected device with PID 00C0; nothing written.".to_string()]
    );

    t.focus("y.exe");
    assert!(t.mouse.sent().is_empty());
    let s = t.switches();
    assert!(s[0].1.as_deref().unwrap().contains("need [device] pid"));
}

#[test]
fn dpi_stages_go_to_volatile_storage() {
    let rules = "[[app]]\nexe = \"x.exe\"\nprofile = \"profiles/stages.toml\"\n";
    let mut t = start("volatile", rules);
    t.focus("x.exe");
    let (volatile, persistent) = {
        let st = t.mouse.state();
        (st.stages[0].clone(), st.stages[1].clone())
    };
    assert_ne!(volatile, default_stages());
    assert_eq!(persistent, default_stages(), "flash copy untouched");
    t.focus("y.exe");
    assert_eq!(t.mouse.state().stages[0], default_stages());
}

#[test]
fn buttons_go_to_volatile_profile_byte() {
    let rules = "[[app]]\nexe = \"x.exe\"\nprofile = \"profiles/buttons.toml\"\n";
    let mut t = start("buttons", rules);
    t.focus("x.exe");
    assert_eq!(t.mouse.state().slots[&(0, 4)], (6, vec![2]), "DPI down");
    t.focus("y.exe");
    assert_eq!(t.mouse.state().slots[&(0, 4)], (1, vec![4]), "restored");
    let profiles = t.mouse.state().button_profiles.clone();
    assert!(!profiles.is_empty());
    assert!(
        profiles.iter().all(|&p| p == 0),
        "only profile byte 0 (volatile), got {profiles:?}"
    );
}

#[test]
fn missing_and_invalid_files_turn_the_feature_off() {
    let mut t = start("files", "this is = = not toml");
    let i = t.info();
    assert!(!i.enabled);
    assert!(i.error.is_some());
    t.focus("cs2.exe");
    assert!(t.mouse.sent().is_empty());
    assert!(t.switches().is_empty());

    std::fs::remove_file(&t.file).unwrap();
    t.core.load_app_profiles();
    let i = t.info();
    assert_eq!((i.enabled, i.error), (false, None));
}

#[test]
fn reload_reports_rules_and_reevaluates() {
    let mut t = start("reload", "");
    t.focus("cs2.exe");
    assert!(t.mouse.sent().is_empty());
    std::fs::write(&t.file, RULES).unwrap();
    let Ok(Reply::AppProfiles(i)) = t.core.handle(Request::ReloadAppProfiles) else {
        panic!("expected AppProfiles");
    };
    assert_eq!((i.enabled, i.rules.len(), i.error), (true, 2, None));
    assert_eq!(
        t.mouse.state().polling_hz,
        500,
        "applied for the app in front"
    );

    // Rules gone: the values from before come back.
    std::fs::write(&t.file, "[[app]]\nexe = 1\n").unwrap();
    let Ok(Reply::AppProfiles(i)) = t.core.handle(Request::ReloadAppProfiles) else {
        panic!("expected AppProfiles");
    };
    assert!(!i.enabled && i.error.is_some());
    assert_eq!(t.mouse.state().polling_hz, 1000);
}

// --- Devices that arrive or leave while an app profile is active ---

const B: &str = r"\\?\hid#vid_1532&pid_00b8&mi_00#b";
const OTHER: &str = r"\\?\hid#vid_1532&pid_00c0&mi_00#o";

impl T {
    /// Plug a fresh Viper at `path` (the core sees it on the next rescan).
    fn plug(&mut self, path: &str) -> FakeMouse {
        self.core.hw().plug(path, VIPER_PID)
    }

    /// Plug `mouse` back in with the state it kept (a wireless mouse whose dongle
    /// was out).
    fn replug(&mut self, path: &str, mouse: &FakeMouse) {
        self.core
            .hw()
            .mice
            .insert(path.into(), (VIPER_PID, mouse.clone()));
        self.core.rescan();
    }

    fn unplug(&mut self, path: &str) {
        self.core.hw().mice.remove(path);
        self.core.rescan();
    }

    fn event_from(&mut self, path: &str) {
        self.core.dispatch(CoreMsg::DeviceEvent(Event::DpiChanged {
            path: path.into(),
            x: 800,
            y: 800,
        }));
    }
}

#[test]
fn arrival_during_app_profile_gets_it() {
    let mut t = start("late-arrival", RULES);
    t.focus("cs2.exe");
    let _ = t.switches();
    let b = t.plug(B);
    t.core.rescan();
    assert_eq!(
        b.state().polling_hz,
        500,
        "new mouse takes the active profile"
    );
    assert_eq!(b.writes(), 1);
    let s = t.switches();
    assert_eq!(s.len(), 1);
    assert_eq!(
        (s[0].0.as_deref(), s[0].1.as_deref()),
        (Some("cs2.toml"), None)
    );
    assert!(
        s[0].2
            .contains(&"Changed polling rate: 1000 Hz -> 500 Hz.".to_string())
    );
    // Nobody is touched again by a later rescan.
    let sent = (t.mouse.sent().len(), b.sent().len());
    t.core.rescan();
    assert_eq!((t.mouse.sent().len(), b.sent().len()), sent);
}

#[test]
fn arrival_without_app_profile_writes_nothing() {
    let mut t = start("late-none", RULES);
    let b = t.plug(B);
    t.core.rescan();
    assert!(b.sent().is_empty(), "no foreground yet");

    t.focus("explorer.exe");
    let c = t.plug(r"\\?\hid#vid_1532&pid_00b8&mi_00#c");
    t.core.rescan();
    assert!(c.sent().is_empty(), "unmatched app, no default");
    assert!(t.switches().is_empty());
}

#[test]
fn arrival_with_other_pid_is_left_alone() {
    let mut t = start("late-pid", RULES);
    t.focus("cs2.exe");
    let _ = t.switches();
    let other = t.core.hw().plug(OTHER, 0x00C0);
    t.core.rescan();
    assert!(other.sent().is_empty());
    assert!(t.switches().is_empty());
}

#[test]
fn switch_back_restores_the_late_device_to_its_arrival_state() {
    let mut t = start("late-restore", RULES);
    t.focus("cs2.exe");
    let b = t.plug(B);
    b.state().polling_hz = 125;
    t.core.rescan();
    assert_eq!(b.state().polling_hz, 500);
    t.focus("explorer.exe");
    assert_eq!(t.mouse.state().polling_hz, 1000);
    assert_eq!(
        b.state().polling_hz,
        125,
        "back to what it had when it arrived"
    );
    assert!(!t.info().restore_pending);
}

#[test]
fn removal_keeps_the_values_from_before_quietly() {
    let mut t = start("late-removal", RULES);
    t.focus("cs2.exe");
    let _b = t.plug(B);
    t.core.rescan();
    t.unplug(B);
    let _ = t.switches();
    t.focus("explorer.exe");
    let s = t.switches();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].1, None, "no error for the unplugged mouse");
    assert!(
        s[0].2
            .iter()
            .any(|l| l.contains("restored when it returns")),
        "{:?}",
        s[0].2
    );
    assert_eq!(t.mouse.state().polling_hz, 1000);
    assert!(t.info().restore_pending, "kept for the unplugged mouse");
}

/// Hardware 2026-11-11: the Viper keeps its volatile settings while its dongle is
/// out, so at re-arrival it still has the app values.
#[test]
fn wireless_replug_during_app_profile_keeps_the_real_before() {
    let mut t = start("late-replug", RULES);
    t.focus("cs2.exe");
    assert_eq!(t.mouse.state().polling_hz, 500);
    t.unplug(A);
    t.replug(A, &t.mouse.clone());
    assert_eq!(t.mouse.state().polling_hz, 500);
    t.focus("explorer.exe");
    assert_eq!(
        t.mouse.state().polling_hz,
        1000,
        "the value from before cs2"
    );
    assert!(!t.info().restore_pending);
}

#[test]
fn device_returning_after_the_app_profile_ended_gets_its_values_back() {
    let mut t = start("late-return", RULES);
    t.focus("cs2.exe");
    t.unplug(A);
    t.focus("explorer.exe");
    let _ = t.switches();
    assert!(t.info().restore_pending);
    // Away, the mouse kept the app value.
    assert_eq!(t.mouse.state().polling_hz, 500);
    t.mouse.state().answers = false;
    t.replug(A, &t.mouse.clone());
    let s = t.switches();
    assert_eq!(s.len(), 1, "one error at return");
    assert!(s[0].1.is_some());
    // It wakes up and talks: the restore is retried and works.
    t.mouse.state().answers = true;
    t.event_from(A);
    assert_eq!(t.mouse.state().polling_hz, 1000);
    let s = t.switches();
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].0.as_deref(), s[0].1.as_deref()), (None, None));
    assert!(!t.info().restore_pending);
    // Done: later events cost nothing.
    let sent = t.mouse.sent().len();
    t.event_from(A);
    assert_eq!(t.mouse.sent().len(), sent);
}

#[test]
fn asleep_device_is_reported_once_and_retried_on_its_next_event() {
    let mut t = start("late-asleep", RULES);
    t.focus("cs2.exe");
    let _ = t.switches();
    let b = t.plug(B);
    b.state().answers = false;
    t.core.rescan();
    let s = t.switches();
    assert_eq!(s.len(), 1, "one error event at arrival");
    assert!(s[0].1.is_some());

    // A later hotplug change retries, quietly while it still fails.
    let sent = b.sent().len();
    let other = t.core.hw().plug(OTHER, 0x00C0);
    t.core.rescan();
    assert!(other.sent().is_empty());
    assert!(b.sent().len() > sent, "retried");
    assert!(t.switches().is_empty(), "no error spam on retries");

    // Events from other devices do not retry it.
    let sent = b.sent().len();
    t.event_from(A);
    assert_eq!(b.sent().len(), sent);

    // The mouse wakes up and sends an event: now it takes the profile.
    b.state().answers = true;
    t.event_from(B);
    assert_eq!(b.state().polling_hz, 500);
    let s = t.switches();
    assert_eq!(s.len(), 1);
    assert_eq!(
        (s[0].0.as_deref(), s[0].1.as_deref()),
        (Some("cs2.toml"), None)
    );

    // Done: later events cost nothing.
    let sent = b.sent().len();
    t.event_from(B);
    assert_eq!(b.sent().len(), sent);

    // Its arrival state comes back on switching away.
    t.focus("explorer.exe");
    assert_eq!(b.state().polling_hz, 1000);
}

#[test]
fn removal_of_a_waiting_device_stops_its_retry() {
    let mut t = start("late-pending-removal", RULES);
    t.focus("cs2.exe");
    let b = t.plug(B);
    b.state().answers = false;
    t.core.rescan();
    t.unplug(B);
    let _ = t.switches();
    let sent = b.sent().len();
    let _other = t.core.hw().plug(OTHER, 0x00C0);
    t.core.rescan();
    assert_eq!(b.sent().len(), sent);
    assert!(t.switches().is_empty());
}

#[test]
fn switch_away_clears_a_pending_retry() {
    let mut t = start("late-pending-switch", RULES);
    t.focus("cs2.exe");
    let b = t.plug(B);
    b.state().answers = false;
    t.core.rescan();
    t.focus("explorer.exe");
    b.state().answers = true;
    let sent = b.sent().len();
    t.event_from(B);
    assert_eq!(b.sent().len(), sent, "no app profile active any more");
    assert_eq!(b.state().polling_hz, 1000);
}

#[test]
fn razer_running_at_arrival_writes_nothing_and_retries() {
    let mut t = start("late-conflict", RULES);
    t.focus("cs2.exe");
    let _ = t.switches();
    t.core.hw().conflict = Some("RazerAppEngine.exe".into());
    let b = t.plug(B);
    t.core.rescan();
    assert!(b.sent().is_empty());
    let s = t.switches();
    assert_eq!(s.len(), 1);
    assert!(s[0].1.as_deref().unwrap().contains("RazerAppEngine.exe"));

    t.core.hw().conflict = None;
    t.event_from(B);
    assert_eq!(b.state().polling_hz, 500);
}
