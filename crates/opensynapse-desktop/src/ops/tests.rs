//! Tests against a fake mouse that answers feature reports from in-memory state.
//! Reply argument layouts are produced with the protocol's own `set_*` builders
//! (a GET reply has the same layout as the matching SET request).

use super::*;
use opensynapse_device_registry::{RAZER_VID, Registry};
use opensynapse_hid::Timing;
use opensynapse_protocol::command::{self, Command};
use opensynapse_protocol::{FEATURE_REPORT_LEN, Report};
use std::cell::RefCell;
use std::time::Duration;

struct State {
    dpi: dpi::Dpi,
    polling_hz: u32,
    sleep: u16,
    period_supported: bool,
    /// Accept writes but keep the old value (read-back mismatch).
    ignore_writes: bool,
    answers: bool,
}

struct FakeMouse {
    state: RefCell<State>,
    pending: RefCell<Option<[u8; FEATURE_REPORT_LEN]>>,
    /// (class, id) of every report sent.
    sent: RefCell<Vec<(u8, u8)>>,
}

impl FakeMouse {
    fn new() -> Self {
        Self {
            state: RefCell::new(State {
                dpi: dpi::Dpi::uniform(800),
                polling_hz: 1000,
                sleep: 300,
                period_supported: true,
                ignore_writes: false,
                answers: true,
            }),
            pending: RefCell::default(),
            sent: RefCell::default(),
        }
    }

    fn writes(&self) -> usize {
        self.sent
            .borrow()
            .iter()
            .filter(|(_, id)| id & 0x80 == 0)
            .count()
    }
}

/// Args of an encoded request (to borrow its layout for a reply).
fn args_of(r: Request) -> Vec<u8> {
    r.encode(0).args().to_vec()
}

/// Reply = the request echoed with `status` and `args` in the argument area.
fn reply(request: &[u8], status: u8, args: &[u8]) -> [u8; FEATURE_REPORT_LEN] {
    let mut out = [0u8; FEATURE_REPORT_LEN];
    out.copy_from_slice(request);
    out[1] = status;
    out[9..9 + args.len()].copy_from_slice(args);
    out
}

impl FeatureIo for &FakeMouse {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError> {
        let req = Report::from_feature_report(buf).unwrap();
        let is = |c: Command| c.class == req.class() && c.id == req.command_id();
        self.sent.borrow_mut().push((req.class(), req.command_id()));
        let mut st = self.state.borrow_mut();
        let ok = |args: Vec<u8>| (2u8, args);
        let (status, args) = if !st.answers {
            (5, vec![])
        } else if is(command::GET_FIRMWARE_VERSION) {
            ok(vec![1, 4])
        } else if is(command::GET_SERIAL) {
            ok(b"PM-TEST".to_vec())
        } else if is(command::GET_DEVICE_MODE) {
            ok(vec![0, 0])
        } else if is(command::GET_BATTERY_LEVEL) {
            ok(vec![0, 255])
        } else if is(command::GET_CHARGING_STATUS) {
            ok(vec![0, 1])
        } else if is(command::GET_DPI) {
            ok(args_of(dpi::set_dpi(dpi::Storage::Volatile, st.dpi)))
        } else if is(command::SET_DPI) {
            if !st.ignore_writes {
                st.dpi = dpi::parse_dpi(req.args()).unwrap();
            }
            ok(vec![])
        } else if is(command::GET_DPI_STAGES) {
            let stages = dpi::DpiStages {
                active: 2,
                stages: vec![
                    dpi::DpiStage {
                        id: 1,
                        dpi: dpi::Dpi::uniform(400),
                    },
                    dpi::DpiStage {
                        id: 2,
                        dpi: dpi::Dpi::uniform(800),
                    },
                ],
            };
            ok(args_of(
                dpi::set_dpi_stages(dpi::Storage::Volatile, &stages).unwrap(),
            ))
        } else if is(command::GET_POLLING_PERIOD) {
            if st.period_supported {
                ok(args_of(polling::set_polling_rate(st.polling_hz).unwrap()))
            } else {
                (5, vec![])
            }
        } else if is(command::SET_POLLING_PERIOD) {
            if !st.ignore_writes {
                st.polling_hz = polling::parse_polling_rate(req.args()).unwrap();
            }
            ok(vec![])
        } else if is(command::GET_HYPER_POLLING) {
            ok(args_of(
                polling::set_hyper_polling(HYPER_PROFILE, st.polling_hz).unwrap(),
            ))
        } else if is(command::GET_SLEEP_TIME) {
            ok(args_of(power::set_sleep_time(st.sleep)))
        } else if is(command::SET_SLEEP_TIME) {
            if !st.ignore_writes {
                st.sleep = power::parse_sleep_time(req.args()).unwrap();
            }
            ok(vec![])
        } else if is(command::GET_BUTTON_ASSIGNMENT) {
            let b = req.args()[1];
            ok(args_of(
                buttons::set_button_assignment(&buttons::mouse_button(1, b, Layer::Normal, b))
                    .unwrap(),
            ))
        } else {
            (5, vec![])
        };
        *self.pending.borrow_mut() = Some(reply(buf, status, &args));
        Ok(())
    }

    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError> {
        buf.copy_from_slice(&self.pending.borrow().expect("GET before SET"));
        Ok(FEATURE_REPORT_LEN)
    }
}

fn session(m: &FakeMouse) -> Session<&FakeMouse> {
    Session::new(
        m,
        Timing {
            between_out: Duration::ZERO,
            out_to_in: Duration::ZERO,
            between_in: Duration::ZERO,
            max_in: 2,
            max_out: 2,
        },
    )
}

/// Viper V3 HyperSpeed: 100–30000 DPI step 50, 125/500/1000 Hz.
fn viper() -> &'static Device {
    Registry::embedded()
        .lookup(RAZER_VID, 0x00B8)
        .expect("Viper V3 HyperSpeed in the registry")
        .device()
}

#[test]
fn snapshot_reads_every_field() {
    let m = FakeMouse::new();
    let snap = read_snapshot(&mut session(&m)).unwrap();
    assert_eq!(snap.firmware.to_string(), "1.04");
    assert_eq!(snap.serial.as_deref(), Ok("PM-TEST"));
    assert_eq!(snap.battery.unwrap().percent(), 100);
    assert_eq!(snap.charging, Ok(ChargingStatus::Charging));
    assert_eq!(snap.dpi, Ok(dpi::Dpi::uniform(800)));
    let stages = snap.stages.unwrap();
    assert_eq!((stages.active, stages.stages.len()), (2, 2));
    assert_eq!(snap.polling, Ok((1000, PollingVariant::Period)));
    assert_eq!(snap.sleep, Ok(300));
    let shown: Vec<String> = snap
        .buttons
        .iter()
        .map(|(_, a)| a.as_ref().unwrap().known().unwrap().to_string())
        .collect();
    assert_eq!(shown, ["mouse button 4", "mouse button 5"]);
    assert_eq!(m.writes(), 0, "reading must never write");
}

#[test]
fn snapshot_fails_fast_when_device_is_silent() {
    let m = FakeMouse::new();
    m.state.borrow_mut().answers = false;
    assert!(read_snapshot(&mut session(&m)).is_err());
    assert_eq!(m.sent.borrow().len(), 1, "only the firmware read is tried");
}

#[test]
fn polling_falls_back_to_hyper() {
    let m = FakeMouse::new();
    m.state.borrow_mut().period_supported = false;
    let got = read_polling(&mut session(&m)).unwrap();
    assert_eq!(got, (1000, PollingVariant::Hyper));
}

#[test]
fn dpi_write_verifies_and_restores() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    let new = Setting::Dpi(dpi::Dpi::uniform(1600));
    let out = guarded_write(&mut s, new, Some(viper()), None, false).unwrap();
    assert_eq!(out.old, Setting::Dpi(dpi::Dpi::uniform(800)));
    assert!(out.verified());
    assert_eq!(m.state.borrow().dpi, dpi::Dpi::uniform(1600));
    // Restore = the same guarded write with the old value.
    let back = guarded_write(&mut s, out.old, Some(viper()), None, false).unwrap();
    assert!(back.verified());
    assert_eq!(m.state.borrow().dpi, dpi::Dpi::uniform(800));
}

#[test]
fn polling_and_sleep_writes() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    let p = Setting::Polling {
        hz: 500,
        variant: PollingVariant::Period,
    };
    let out = guarded_write(&mut s, p, Some(viper()), None, false).unwrap();
    assert!(out.verified());
    assert_eq!(m.state.borrow().polling_hz, 500);
    let out = guarded_write(&mut s, Setting::Sleep(600), Some(viper()), None, false).unwrap();
    assert_eq!(
        (out.old, out.now),
        (Setting::Sleep(300), Setting::Sleep(600))
    );
}

#[test]
fn read_back_mismatch_is_reported() {
    let m = FakeMouse::new();
    m.state.borrow_mut().ignore_writes = true;
    let new = Setting::Dpi(dpi::Dpi::uniform(1600));
    let out = guarded_write(&mut session(&m), new, Some(viper()), None, false).unwrap();
    assert!(!out.verified());
    assert_eq!(out.now, out.old);
}

#[test]
fn conflict_blocks_writes_until_confirmed() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    let new = Setting::Sleep(600);
    let err = guarded_write(
        &mut s,
        new,
        Some(viper()),
        Some("RazerAppEngine.exe"),
        false,
    )
    .unwrap_err();
    assert!(
        err.message.contains("RazerAppEngine.exe"),
        "{}",
        err.message
    );
    assert_eq!(err.old, None);
    assert!(m.sent.borrow().is_empty(), "nothing may be sent");
    let ok = guarded_write(&mut s, new, Some(viper()), Some("RazerAppEngine.exe"), true);
    assert!(ok.unwrap().verified());
}

#[test]
fn invalid_values_are_never_sent() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    for bad in [
        Setting::Dpi(dpi::Dpi::uniform(50)),
        Setting::Dpi(dpi::Dpi::uniform(30050)),
        Setting::Dpi(dpi::Dpi::uniform(825)),
        Setting::Polling {
            hz: 8000,
            variant: PollingVariant::Period,
        },
        Setting::Sleep(0),
    ] {
        assert!(
            guarded_write(&mut s, bad, Some(viper()), None, false).is_err(),
            "{bad} accepted"
        );
    }
    assert!(
        validate(&Setting::Sleep(300), None).is_err(),
        "unknown device"
    );
    assert!(m.sent.borrow().is_empty());
}

#[test]
fn failed_write_keeps_old_value_for_restore() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    // The fake answers "not supported" to the hyper SET, after the GET worked.
    let new = Setting::Polling {
        hz: 500,
        variant: PollingVariant::Hyper,
    };
    let err = guarded_write(&mut s, new, Some(viper()), None, false).unwrap_err();
    assert_eq!(
        err.old,
        Some(Setting::Polling {
            hz: 1000,
            variant: PollingVariant::Hyper
        })
    );
}
