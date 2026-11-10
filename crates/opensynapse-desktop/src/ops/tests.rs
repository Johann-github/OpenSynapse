//! Tests against a fake mouse that answers feature reports from in-memory state.
//! Reply argument layouts are produced with the protocol's own `set_*` builders
//! (a GET reply has the same layout as the matching SET request).

use super::*;
use opensynapse_device_registry::{RAZER_VID, Registry};
use opensynapse_hid::Timing;
use opensynapse_protocol::command::{self, Command};
use opensynapse_protocol::{FEATURE_REPORT_LEN, Report};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

struct State {
    dpi: dpi::Dpi,
    polling_hz: u32,
    sleep: u16,
    period_supported: bool,
    /// Accept writes but keep the old value (read-back mismatch).
    ignore_writes: bool,
    answers: bool,
    /// (mode, button) -> (function, data); Viper defaults: button N -> mouse
    /// button N, DPI button -> DPI function.
    slots: BTreeMap<(u8, u8), (u8, Vec<u8>)>,
    /// Stage lists by storage byte.
    stages: [dpi::DpiStages; 2],
}

/// Viper V3 HyperSpeed button IDs in 02/8E order (observed).
const VIPER_BUTTONS: [u8; 8] = [1, 2, 3, 4, 5, 96, 9, 10];

fn default_stages() -> dpi::DpiStages {
    stages_from(&[400, 800], 1).unwrap()
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
                slots: [0u8, 1]
                    .into_iter()
                    .flat_map(|m| {
                        VIPER_BUTTONS.map(|b| {
                            let v = if b == 96 { (6, vec![6]) } else { (1, vec![b]) };
                            ((m, b), v)
                        })
                    })
                    .collect(),
                stages: [default_stages(), default_stages()],
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
            let storage = req.args()[0];
            let st_val = if storage == 1 {
                dpi::Storage::Persistent
            } else {
                dpi::Storage::Volatile
            };
            let stages = st.stages[usize::from(storage)].clone();
            ok(args_of(dpi::set_dpi_stages(st_val, &stages).unwrap()))
        } else if is(command::SET_DPI_STAGES) {
            if !st.ignore_writes {
                let new = dpi::parse_dpi_stages(req.args()).unwrap();
                // Setting the active stage also applies its DPI (observed).
                st.dpi = new.stages[new.active_index().unwrap()].dpi;
                st.stages[usize::from(req.args()[0])] = new;
            }
            ok(vec![])
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
            let (p, b, m) = (req.args()[0], req.args()[1], req.args()[2]);
            match st.slots.get(&(m, b)) {
                Some((function, data)) => {
                    // Reply byte 2 is not the mode (observed); the fake sends 1.
                    let mut args = vec![p, b, 1, *function, data.len() as u8];
                    args.extend_from_slice(data);
                    ok(args)
                }
                None => (5, vec![]),
            }
        } else if is(command::SET_BUTTON_ASSIGNMENT) {
            let a = req.args();
            let (b, m, len) = (a[1], a[2], usize::from(a[4]));
            if !st.ignore_writes && st.slots.contains_key(&(m, b)) {
                st.slots.insert((m, b), (a[3], a[5..5 + len].to_vec()));
            }
            ok(vec![])
        } else if is(command::GET_BUTTON_ASSIGNMENT_LIST) {
            let (p, m) = (req.args()[0], req.args()[1]);
            let mut args = vec![p, m, 0];
            for b in VIPER_BUTTONS {
                let (function, data) = &st.slots[&(m, b)];
                let mut rec = [0u8; 8];
                rec[..3].copy_from_slice(&[b, *function, data.len() as u8]);
                rec[3..3 + data.len()].copy_from_slice(data);
                args.extend_from_slice(&rec);
            }
            ok(args)
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

const NO_OPTS: WriteOpts = WriteOpts {
    confirmed: false,
    allow_primary: false,
    restore: false,
};
const RESTORE: WriteOpts = WriteOpts {
    restore: true,
    ..NO_OPTS
};

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
    let normal = snap.buttons_normal.unwrap();
    let ids: Vec<u8> = normal.iter().map(|a| a.button).collect();
    assert_eq!(ids, VIPER_BUTTONS);
    assert!(normal.iter().all(|a| a.mode == Mode::Normal));
    // The DPI button's factory record is function 6 with data [6] (observed), not
    // the "DPI up" layout, so the editor shows it as Other.
    assert_eq!((normal[5].function, normal[5].known()), (6, None));
    assert_eq!(
        ActionEdit::from_assignment(&normal[5]).kind,
        ActionKind::Other
    );
    let hyper = snap.buttons_hypershift.unwrap();
    assert!(hyper.iter().all(|a| a.mode == Mode::HyperShift));
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
    let out = guarded_write(&mut s, new, Some(viper()), None, NO_OPTS).unwrap();
    assert_eq!(out.old, Setting::Dpi(dpi::Dpi::uniform(800)));
    assert!(out.verified());
    assert_eq!(m.state.borrow().dpi, dpi::Dpi::uniform(1600));
    // Restore = the same guarded write with the old value.
    let back = guarded_write(&mut s, out.old, Some(viper()), None, RESTORE).unwrap();
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
    let out = guarded_write(&mut s, p, Some(viper()), None, NO_OPTS).unwrap();
    assert!(out.verified());
    assert_eq!(m.state.borrow().polling_hz, 500);
    let out = guarded_write(&mut s, Setting::Sleep(600), Some(viper()), None, NO_OPTS).unwrap();
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
    let out = guarded_write(&mut session(&m), new, Some(viper()), None, NO_OPTS).unwrap();
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
        new.clone(),
        Some(viper()),
        Some("RazerAppEngine.exe"),
        NO_OPTS,
    )
    .unwrap_err();
    assert!(
        err.message.contains("RazerAppEngine.exe"),
        "{}",
        err.message
    );
    assert_eq!(err.old, None);
    assert!(m.sent.borrow().is_empty(), "nothing may be sent");
    let confirmed = WriteOpts {
        confirmed: true,
        ..NO_OPTS
    };
    let ok = guarded_write(
        &mut s,
        new,
        Some(viper()),
        Some("RazerAppEngine.exe"),
        confirmed,
    );
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
            guarded_write(&mut s, bad.clone(), Some(viper()), None, NO_OPTS).is_err(),
            "{bad} accepted"
        );
    }
    assert!(
        validate(&Setting::Sleep(300), None, NO_OPTS).is_err(),
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
    let err = guarded_write(&mut s, new, Some(viper()), None, NO_OPTS).unwrap_err();
    assert_eq!(
        err.old,
        Some(Setting::Polling {
            hz: 1000,
            variant: PollingVariant::Hyper
        })
    );
}

fn slot(m: &FakeMouse, mode: Mode, button: u8) -> (u8, Vec<u8>) {
    m.state.borrow().slots[&(mode as u8, button)].clone()
}

#[test]
fn button_write_hits_one_slot_and_restores() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    let key = ActionEdit {
        kind: ActionKind::Key,
        usage: 0x04,
        modifiers: 0x01,
    };
    let new = Setting::Button(key.to_assignment(4, Mode::HyperShift).unwrap());
    let out = guarded_write(&mut s, new, Some(viper()), None, NO_OPTS).unwrap();
    assert!(out.verified(), "{out:?}");
    assert_eq!(
        out.old,
        Setting::Button(buttons::mouse_button(1, 4, Mode::HyperShift, 4))
    );
    assert_eq!(
        out.new.to_string(),
        "Side button 4 (HyperShift): key ctrl+a"
    );
    assert_eq!(slot(&m, Mode::HyperShift, 4), (2, vec![0x01, 0x04]));
    assert_eq!(
        slot(&m, Mode::Normal, 4),
        (1, vec![4]),
        "normal slot untouched"
    );
    let back = guarded_write(&mut s, out.old, Some(viper()), None, RESTORE).unwrap();
    assert!(back.verified());
    assert_eq!(slot(&m, Mode::HyperShift, 4), (1, vec![4]));
}

#[test]
fn left_and_right_click_need_opt_in() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    for b in PRIMARY_BUTTONS {
        let new = Setting::Button(buttons::dpi_up(1, b, Mode::Normal));
        let err = guarded_write(&mut s, new, Some(viper()), None, NO_OPTS).unwrap_err();
        assert!(err.message.contains("protected"), "{}", err.message);
    }
    assert!(m.sent.borrow().is_empty(), "nothing may be sent");
    let allow = WriteOpts {
        allow_primary: true,
        ..NO_OPTS
    };
    let new = Setting::Button(buttons::double_click(1, 2, Mode::Normal, 1));
    let out = guarded_write(&mut s, new, Some(viper()), None, allow).unwrap();
    assert!(out.verified());
    // Restore needs no opt-in: it writes back what the device held.
    let back = guarded_write(&mut s, out.old, Some(viper()), None, RESTORE).unwrap();
    assert!(back.verified());
    assert_eq!(slot(&m, Mode::Normal, 2), (1, vec![2]));
}

#[test]
fn only_known_actions_are_written_except_restore() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    let raw = ButtonAssignment {
        profile: 1,
        button: 4,
        mode: Mode::Normal,
        function: 3,
        data: vec![9],
    };
    let bad = [
        raw.clone(),
        buttons::mouse_button(1, 4, Mode::Normal, 6),
        buttons::double_click(1, 4, Mode::Normal, 2),
        buttons::keyboard(1, 4, Mode::Normal, 0, 0x00),
        buttons::keyboard(1, 4, Mode::Normal, 0x10, 0x04),
    ];
    for a in bad {
        let r = guarded_write(&mut s, Setting::Button(a), Some(viper()), None, NO_OPTS);
        assert!(r.is_err());
    }
    assert!(m.sent.borrow().is_empty());
    let out = guarded_write(&mut s, Setting::Button(raw), Some(viper()), None, RESTORE).unwrap();
    assert!(out.verified());
    assert_eq!(slot(&m, Mode::Normal, 4), (3, vec![9]));
}

#[test]
fn stage_write_volatile_and_persistent() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    let stages = stages_from(&[400, 800, 1600], 2).unwrap();
    let new = Setting::DpiStages {
        storage: dpi::Storage::Volatile,
        stages: stages.clone(),
    };
    let out = guarded_write(&mut s, new, Some(viper()), None, NO_OPTS).unwrap();
    assert!(out.verified());
    assert_eq!(out.new.to_string(), "DPI stages 400 800 [1600] (volatile)");
    assert_eq!(m.state.borrow().stages[0], stages);
    assert_eq!(
        m.state.borrow().stages[1],
        default_stages(),
        "persistent untouched"
    );
    assert_eq!(m.state.borrow().dpi, dpi::Dpi::uniform(1600));
    guarded_write(&mut s, out.old, Some(viper()), None, RESTORE).unwrap();
    assert_eq!(m.state.borrow().stages[0], default_stages());

    let new = Setting::DpiStages {
        storage: dpi::Storage::Persistent,
        stages: stages_from(&[1000], 0).unwrap(),
    };
    let out = guarded_write(&mut s, new, Some(viper()), None, NO_OPTS).unwrap();
    assert!(out.verified());
    assert_eq!(m.state.borrow().stages[1].stages.len(), 1);
    assert_eq!(m.state.borrow().stages[0], default_stages());
}

#[test]
fn invalid_stages_are_never_sent() {
    let m = FakeMouse::new();
    let mut s = session(&m);
    let mut dup = stages_from(&[400, 800], 0).unwrap();
    dup.stages[1].id = 1;
    let mut lost = stages_from(&[400, 800], 0).unwrap();
    lost.active = 7;
    for stages in [
        stages_from(&[400, 825], 0).unwrap(),
        stages_from(&[50], 0).unwrap(),
        dpi::DpiStages {
            active: 1,
            stages: vec![],
        },
        dup,
        lost,
    ] {
        let new = Setting::DpiStages {
            storage: dpi::Storage::Volatile,
            stages,
        };
        assert!(guarded_write(&mut s, new, Some(viper()), None, NO_OPTS).is_err());
    }
    assert!(m.sent.borrow().is_empty());
}

#[test]
fn stage_helpers() {
    let s = stages_from(&[400, 800, 1600], 1).unwrap();
    assert_eq!(s.active, 2);
    let ids: Vec<u8> = s.stages.iter().map(|st| st.id).collect();
    assert_eq!(ids, [1, 2, 3]);
    assert!(stages_from(&[], 0).is_err());
    assert!(stages_from(&[400], 1).is_err());
    assert!(stages_from(&[400; 12], 0).is_err());
    assert_eq!(stage_limit(2), DEFAULT_MAX_STAGES);
    assert_eq!(stage_limit(7), 7);
    assert_eq!(stage_limit(20), dpi::MAX_DPI_STAGES);
    assert_eq!(snap_dpi(824, 100, 30000, 50), 800);
    assert_eq!(snap_dpi(825, 100, 30000, 50), 850);
    assert_eq!(snap_dpi(10, 100, 30000, 50), 100);
    assert_eq!(snap_dpi(40000, 100, 30000, 50), 30000);
    assert_eq!(snap_dpi(1003, 100, 1010, 50), 1000, "never past max");
}

#[test]
fn action_edit_round_trip() {
    for kind in ActionKind::choices() {
        let e = ActionEdit {
            kind,
            usage: 0x2C,
            modifiers: 0x06,
        };
        match e.to_assignment(5, Mode::HyperShift) {
            Some(a) => {
                assert_eq!((a.profile, a.button, a.mode), (1, 5, Mode::HyperShift));
                assert!(check_action(&a).is_ok(), "{kind:?}");
                let back = ActionEdit::from_assignment(&a);
                assert_eq!(back.kind, kind);
                if kind == ActionKind::Key {
                    assert_eq!(back, e);
                }
            }
            None => assert_eq!(kind, ActionKind::Other),
        }
    }
    // Bytes come from the protocol builders.
    let e = ActionEdit {
        kind: ActionKind::DpiUp,
        usage: 4,
        modifiers: 0,
    };
    assert_eq!(
        e.to_assignment(4, Mode::Normal),
        Some(buttons::dpi_up(1, 4, Mode::Normal))
    );
    // Unknown functions, unnamed keys, right-side modifiers and mouse buttons the
    // editor does not offer show as Other.
    for a in [
        ButtonAssignment {
            profile: 1,
            button: 4,
            mode: Mode::Normal,
            function: 0,
            data: vec![],
        },
        buttons::keyboard(1, 4, Mode::Normal, 0, 0x50),
        buttons::keyboard(1, 4, Mode::Normal, 0x20, 0x04),
        buttons::mouse_button(1, 4, Mode::Normal, 7),
        buttons::double_click(1, 4, Mode::Normal, 2),
    ] {
        assert_eq!(
            ActionEdit::from_assignment(&a).kind,
            ActionKind::Other,
            "{a}"
        );
    }
    let labels: Vec<String> = ActionKind::choices()
        .into_iter()
        .map(ActionKind::label)
        .collect();
    assert_eq!(labels[0], "Left click");
    assert_eq!(labels.last().unwrap(), "Other (read-only)");
}

#[test]
fn key_choices_cover_hid_usage_names() {
    let keys = key_choices();
    assert_eq!(keys.len(), 26 + 10 + 5 + 12);
    assert_eq!(keys[0], (0x04, "a".to_string()));
    assert!(keys.contains(&(0x45, "f12".to_string())));
    let bits: Vec<u8> = modifier_choices().iter().map(|&(b, _)| b).collect();
    assert_eq!(bits, [0x01, 0x02, 0x04, 0x08]);
}

#[test]
fn rows_merge_both_modes() {
    let n = vec![
        buttons::mouse_button(1, 1, Mode::Normal, 1),
        buttons::mouse_button(1, 4, Mode::Normal, 4),
    ];
    let h = vec![
        buttons::mouse_button(1, 4, Mode::HyperShift, 5),
        buttons::mouse_button(1, 7, Mode::HyperShift, 7),
    ];
    let rows = button_rows(&n, &h);
    let ids: Vec<u8> = rows.iter().map(|r| r.button).collect();
    assert_eq!(ids, [1, 4, 7]);
    assert_eq!(rows[0].slot(Mode::HyperShift), None);
    assert_eq!(rows[1].slot(Mode::HyperShift), Some(&h[0]));
    assert_eq!(rows[2].slot(Mode::Normal), None);
    assert_eq!(button_name(96), "DPI button");
    assert_eq!(button_name(9), "Wheel up");
    assert_eq!(button_name(42), "Button 42");
}
