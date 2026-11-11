//! Core tests against fake mice. No real hardware, no pipe.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use opensynapse_ipc::msg::{
    DeviceRef, ErrorKind, Event, Reply, Request, WireSetting, WireWriteOpts,
};
use opensynapse_protocol::dpi::Dpi;

use super::{Core, CoreMsg};
use crate::events::NoEvents;
use crate::hardware::fake::{FakeHardware, VIPER_PID};

const A: &str = r"\\?\hid#vid_1532&pid_00b8&mi_00#a";
const B: &str = r"\\?\hid#vid_1532&pid_00b8&mi_00#b";

fn core() -> (Core<FakeHardware>, Receiver<CoreMsg>) {
    let (tx, rx) = channel();
    let c = Core::new(
        FakeHardware::default(),
        Box::new(NoEvents),
        tx,
        Arc::new(AtomicBool::new(false)),
        Duration::ZERO,
    );
    (c, rx)
}

/// Core with one Viper plugged in and scanned.
fn with_viper() -> (Core<FakeHardware>, opensynapse_device::fake::FakeMouse) {
    let (mut c, _) = core();
    let m = c.hw().plug(A, VIPER_PID);
    c.rescan();
    (c, m)
}

fn subscribe(c: &mut Core<FakeHardware>) -> Receiver<Event> {
    let (tx, rx) = channel();
    assert!(!c.dispatch(CoreMsg::Subscribe(tx)));
    rx
}

fn write_dpi(v: u16, confirmed: bool) -> Request {
    Request::Write {
        device: DeviceRef::Pid(VIPER_PID),
        setting: WireSetting::Dpi(Dpi::uniform(v)),
        opts: WireWriteOpts {
            confirmed,
            ..Default::default()
        },
    }
}

#[test]
fn ping() {
    let (mut c, _) = core();
    assert!(matches!(c.handle(Request::Ping), Ok(Reply::Pong { .. })));
}

#[test]
fn list_devices() {
    let (mut c, _) = with_viper();
    c.hw().conflict = Some("RazerAppEngine.exe".into());
    let Ok(Reply::Devices { list, conflict }) = c.handle(Request::ListDevices) else {
        panic!("expected Devices");
    };
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].path, A);
    assert_eq!(list[0].pid, VIPER_PID);
    assert!(list[0].supported);
    assert_eq!(conflict.as_deref(), Some("RazerAppEngine.exe"));
}

#[test]
fn read_snapshot_reuses_session() {
    let (mut c, m) = with_viper();
    for _ in 0..2 {
        let Ok(Reply::Snapshot(s)) = c.handle(Request::ReadSnapshot {
            device: DeviceRef::Path(A.into()),
        }) else {
            panic!("expected Snapshot");
        };
        assert_eq!(s.dpi, Ok(Dpi::uniform(800)));
        assert_eq!(s.serial.as_deref(), Ok("PM-TEST"));
    }
    assert_eq!(c.hw().opens, 1);
    assert_eq!(m.writes(), 0, "a read never writes");
}

#[test]
fn failed_snapshot_drops_session() {
    let (mut c, m) = with_viper();
    m.state().answers = false;
    let e = c
        .handle(Request::ReadSnapshot {
            device: DeviceRef::Pid(VIPER_PID),
        })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Device);
    assert_eq!(c.session_count(), 0);
}

/// A sleeping mouse (status Timeout to every report) keeps its class over IPC.
#[test]
fn asleep_snapshot_is_no_response() {
    let (mut c, m) = with_viper();
    m.state().asleep = true;
    let e = c
        .handle(Request::ReadSnapshot {
            device: DeviceRef::Pid(VIPER_PID),
        })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NoResponse, "{}", e.message);
    assert!(opensynapse_hid::is_no_response_text(&e.message));
    assert_eq!(c.session_count(), 0);
}

#[test]
fn verified_write_returns_old() {
    let (mut c, m) = with_viper();
    let Ok(Reply::Written(Ok(done))) = c.handle(write_dpi(1600, false)) else {
        panic!("expected a finished write");
    };
    assert_eq!(done.old, WireSetting::Dpi(Dpi::uniform(800)));
    assert_eq!(done.now, WireSetting::Dpi(Dpi::uniform(1600)));
    assert!(done.verified);
    assert_eq!(m.state().dpi, Dpi::uniform(1600));
}

#[test]
fn write_refused_while_razer_runs_unless_confirmed() {
    let (mut c, m) = with_viper();
    c.hw().conflict = Some("RazerAppEngine.exe".into());
    let e = c.handle(write_dpi(1600, false)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert_eq!(m.writes(), 0);
    assert!(m.sent().is_empty(), "refused before touching the device");

    assert!(matches!(
        c.handle(write_dpi(1600, true)),
        Ok(Reply::Written(Ok(_)))
    ));
    assert_eq!(m.state().dpi, Dpi::uniform(1600));
}

#[test]
fn invalid_value_is_a_failed_write() {
    let (mut c, m) = with_viper();
    let Ok(Reply::Written(Err(f))) = c.handle(write_dpi(7, false)) else {
        panic!("expected a refused write");
    };
    assert_eq!(f.old, None);
    assert_eq!(m.writes(), 0);
}

#[test]
fn unknown_device_refused() {
    let (mut c, _) = core();
    let m = c.hw().plug(A, 0xFFFF);
    c.rescan();
    let e = c
        .handle(Request::Write {
            device: DeviceRef::Pid(0xFFFF),
            setting: WireSetting::Sleep(300),
            opts: WireWriteOpts::default(),
        })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    assert!(m.sent().is_empty());
    assert_eq!(c.hw().opens, 0);
}

#[test]
fn pid_missing_and_ambiguous() {
    let (mut c, _) = core();
    let missing = c
        .handle(Request::ReadSnapshot {
            device: DeviceRef::Pid(VIPER_PID),
        })
        .unwrap_err();
    assert_eq!(missing.kind, ErrorKind::NoSuchDevice);

    c.hw().plug(A, VIPER_PID);
    c.hw().plug(B, VIPER_PID);
    c.rescan();
    let amb = c
        .handle(Request::ReadSnapshot {
            device: DeviceRef::Pid(VIPER_PID),
        })
        .unwrap_err();
    assert_eq!(amb.kind, ErrorKind::Ambiguous);
    // A path still picks one.
    assert!(
        c.handle(Request::ReadSnapshot {
            device: DeviceRef::Path(B.into()),
        })
        .is_ok()
    );
    let bad_path = c
        .handle(Request::ReadSnapshot {
            device: DeviceRef::Path("nope".into()),
        })
        .unwrap_err();
    assert_eq!(bad_path.kind, ErrorKind::NoSuchDevice);
}

#[test]
fn rescan_reports_arrival_and_removal() {
    let (mut c, _) = core();
    let events = subscribe(&mut c);

    c.hw().plug(A, VIPER_PID);
    assert!(!c.dispatch(CoreMsg::Rescan));
    let Ok(Event::DeviceArrived(info)) = events.try_recv() else {
        panic!("expected DeviceArrived");
    };
    assert_eq!(info.path, A);

    // Nothing changed: no event.
    c.rescan();
    assert!(events.try_recv().is_err());

    // Open a session, then unplug: session dropped, removal reported.
    c.handle(Request::ReadSnapshot {
        device: DeviceRef::Path(A.into()),
    })
    .unwrap();
    assert_eq!(c.session_count(), 1);
    c.hw().mice.clear();
    c.rescan();
    assert_eq!(
        events.try_recv(),
        Ok(Event::DeviceRemoved {
            path: A.into(),
            pid: VIPER_PID
        })
    );
    assert_eq!(c.session_count(), 0);
}

#[test]
fn rescan_clears_pending_flag() {
    let (tx, _rx) = channel();
    let pending = Arc::new(AtomicBool::new(true));
    let mut c = Core::new(
        FakeHardware::default(),
        Box::new(NoEvents),
        tx,
        pending.clone(),
        Duration::ZERO,
    );
    c.dispatch(CoreMsg::Rescan);
    assert!(!pending.load(Ordering::SeqCst));
}

#[test]
fn dead_subscriber_pruned() {
    let (mut c, _) = core();
    let live = subscribe(&mut c);
    drop(subscribe(&mut c));
    assert_eq!(c.subscriber_count(), 2);
    c.broadcast(Event::Error {
        message: "x".into(),
    });
    assert_eq!(c.subscriber_count(), 1);
    assert!(live.try_recv().is_ok());
}

#[test]
fn device_events_reach_subscribers() {
    let (mut c, _) = core();
    let events = subscribe(&mut c);
    let ev = Event::DpiChanged {
        path: A.into(),
        x: 800,
        y: 800,
    };
    assert!(!c.dispatch(CoreMsg::DeviceEvent(ev.clone())));
    assert_eq!(events.try_recv(), Ok(ev));
}

fn apply_req(toml: &str, dry_run: bool, confirmed: bool) -> Request {
    Request::ApplyProfile {
        device: DeviceRef::Pid(VIPER_PID),
        toml: toml.into(),
        dry_run,
        all: false,
        opts: WireWriteOpts {
            confirmed,
            ..Default::default()
        },
    }
}

/// The fake's 1000 Hz becomes 500 Hz; everything else already matches.
const POLLING_500: &str =
    "[device]\npid = \"00B8\"\n[polling]\nhz = 500\n[power]\nsleep_seconds = 300\n";

#[test]
fn apply_profile_writes_and_returns_restore() {
    let (mut c, m) = with_viper();
    let Ok(Reply::ProfileApplied {
        lines,
        restore_toml,
        error,
    }) = c.handle(apply_req(POLLING_500, false, false))
    else {
        panic!("expected ProfileApplied");
    };
    assert_eq!(error, None);
    assert_eq!(m.state().polling_hz, 500);
    assert_eq!(m.writes(), 1, "unchanged sleep time is skipped");
    assert_eq!(lines[0], "unchanged: sleep time (300 s)");
    assert!(lines.contains(&"Changed polling rate: 1000 Hz -> 500 Hz.".to_string()));
    let restore = restore_toml.unwrap();
    assert!(
        restore.starts_with("# OpenSynapse profile: values before applying to "),
        "{restore}"
    );
    assert!(restore.contains("[polling]\nhz = 1000\n"), "{restore}");
    assert!(!restore.contains("[power]"), "only what was written");

    // Applying the restore profile through the agent undoes it.
    let Ok(Reply::ProfileApplied { error: None, .. }) = c.handle(apply_req(&restore, false, false))
    else {
        panic!("restore failed");
    };
    assert_eq!(m.state().polling_hz, 1000);
}

#[test]
fn apply_profile_dry_run_reads_only() {
    let (mut c, m) = with_viper();
    c.hw().conflict = Some("RazerAppEngine.exe".into());
    let Ok(Reply::ProfileApplied {
        lines,
        restore_toml,
        error,
    }) = c.handle(apply_req(POLLING_500, true, false))
    else {
        panic!("expected ProfileApplied");
    };
    assert_eq!((m.writes(), error), (0, None));
    assert!(restore_toml.is_some());
    assert_eq!(lines.last().unwrap(), "Run again with --apply to write.");
}

#[test]
fn apply_profile_refused_while_razer_runs_unless_confirmed() {
    let (mut c, m) = with_viper();
    c.hw().conflict = Some("RazerAppEngine.exe".into());
    let e = c.handle(apply_req(POLLING_500, false, false)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert!(e.message.contains("RazerAppEngine.exe"), "{}", e.message);
    assert!(m.sent().is_empty(), "refused before touching the device");

    assert!(matches!(
        c.handle(apply_req(POLLING_500, false, true)),
        Ok(Reply::ProfileApplied { error: None, .. })
    ));
    assert_eq!(m.state().polling_hz, 500);
}

#[test]
fn apply_profile_bad_input() {
    let (mut c, m) = with_viper();
    let e = c.handle(apply_req("[dpi", true, false)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::BadRequest);
    let e = c
        .handle(apply_req(
            &POLLING_500.replace("00B8", "00B9"),
            false,
            false,
        ))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::BadRequest);
    assert!(e.message.contains("is for PID 00B9"), "{}", e.message);
    assert_eq!(m.writes(), 0);
}

#[test]
fn apply_profile_failed_write_keeps_restore() {
    let (mut c, m) = with_viper();
    m.state().ignore_writes = true;
    let Ok(Reply::ProfileApplied {
        lines,
        restore_toml,
        error,
    }) = c.handle(apply_req(POLLING_500, false, false))
    else {
        panic!("expected ProfileApplied");
    };
    assert_eq!(
        error.as_deref(),
        Some("polling rate reads back as 1000 Hz, not the value written")
    );
    assert!(lines.last().unwrap().starts_with("Stopped at write 1 of 1"));
    assert!(restore_toml.is_some());
    assert_eq!(
        c.session_count(),
        0,
        "session dropped after a device failure"
    );
}

#[test]
fn shutdown_replies_and_stops_run() {
    let (mut c, rx) = with_viper_and_rx();
    let events = subscribe(&mut c);
    let (reply_tx, reply_rx) = channel();
    // Queue the request on the core's own channel, then run.
    c.tx.send(CoreMsg::Request {
        req: Request::Shutdown,
        reply: reply_tx,
    })
    .unwrap();
    c.run(&rx);
    assert!(matches!(reply_rx.try_recv(), Ok(Ok(Reply::ShuttingDown))));
    assert_eq!(events.try_recv(), Ok(Event::ShuttingDown));
    assert_eq!(c.subscriber_count(), 0);
}

fn with_viper_and_rx() -> (Core<FakeHardware>, Receiver<CoreMsg>) {
    let (mut c, rx) = core();
    c.hw().plug(A, VIPER_PID);
    c.rescan();
    (c, rx)
}

#[test]
fn list_rescans_without_hotplug() {
    let (mut c, _) = core();
    c.set_rescan_on_list(true);
    c.hw().plug(A, VIPER_PID);
    let Ok(Reply::Devices { list, .. }) = c.handle(Request::ListDevices) else {
        panic!("expected Devices");
    };
    assert_eq!(list.len(), 1);
}

#[test]
fn asleep_apply_is_no_response() {
    let (mut c, m) = with_viper();
    m.state().asleep = true;
    let e = c.handle(apply_req(POLLING_500, false, false)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NoResponse, "{}", e.message);
    assert_eq!(m.writes(), 0);
}
