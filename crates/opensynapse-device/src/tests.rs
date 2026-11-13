//! Tests against the fake mouse (`fake.rs`).

use super::*;
use crate::fake::{FakeMouse, VIPER_BUTTONS, default_stages};
use opensynapse_device_registry::{RAZER_VID, Registry};
use opensynapse_hid::Timing;
use std::time::Duration;

const NO_OPTS: WriteOpts = WriteOpts {
    confirmed: false,
    allow_primary: false,
    restore: false,
};
const RESTORE: WriteOpts = WriteOpts {
    restore: true,
    ..NO_OPTS
};

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
    m.state().answers = false;
    assert!(read_snapshot(&mut session(&m)).is_err());
    assert_eq!(m.sent().len(), 1, "only the firmware read is tried");
}

#[test]
fn polling_falls_back_to_hyper() {
    let m = FakeMouse::new();
    m.state().period_supported = false;
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
    assert_eq!(m.state().dpi, dpi::Dpi::uniform(1600));
    // Restore = the same guarded write with the old value.
    let back = guarded_write(&mut s, out.old, Some(viper()), None, RESTORE).unwrap();
    assert!(back.verified());
    assert_eq!(m.state().dpi, dpi::Dpi::uniform(800));
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
    assert_eq!(m.state().polling_hz, 500);
    let out = guarded_write(&mut s, Setting::Sleep(600), Some(viper()), None, NO_OPTS).unwrap();
    assert_eq!(
        (out.old, out.now),
        (Setting::Sleep(300), Setting::Sleep(600))
    );
}

#[test]
fn read_back_mismatch_is_reported() {
    let m = FakeMouse::new();
    m.state().ignore_writes = true;
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
    assert!(m.sent().is_empty(), "nothing may be sent");
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
    assert!(m.sent().is_empty());
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
    m.state().slots[&(mode as u8, button)].clone()
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
    assert!(m.sent().is_empty(), "nothing may be sent");
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
    assert!(m.sent().is_empty());
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
    assert_eq!(m.state().stages[0], stages);
    assert_eq!(
        m.state().stages[1],
        default_stages(),
        "persistent untouched"
    );
    assert_eq!(m.state().dpi, dpi::Dpi::uniform(1600));
    guarded_write(&mut s, out.old, Some(viper()), None, RESTORE).unwrap();
    assert_eq!(m.state().stages[0], default_stages());

    let new = Setting::DpiStages {
        storage: dpi::Storage::Persistent,
        stages: stages_from(&[1000], 0).unwrap(),
    };
    let out = guarded_write(&mut s, new, Some(viper()), None, NO_OPTS).unwrap();
    assert!(out.verified());
    assert_eq!(m.state().stages[1].stages.len(), 1);
    assert_eq!(m.state().stages[0], default_stages());
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
    assert!(m.sent().is_empty());
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
    for (a, kind) in [
        (buttons::dpi_down(1, 4, Mode::Normal), ActionKind::DpiDown),
        (buttons::off(1, 4, Mode::Normal), ActionKind::Off),
    ] {
        let e = ActionEdit::from_assignment(&a);
        assert_eq!(e.kind, kind);
        assert_eq!(e.to_assignment(4, Mode::Normal), Some(a));
    }
    // Unknown functions, unnamed keys, right-side modifiers and mouse buttons the
    // editor does not offer show as Other.
    for a in [
        ButtonAssignment {
            profile: 1,
            button: 4,
            mode: Mode::Normal,
            function: 0,
            data: vec![1],
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

#[test]
fn action_edit_profile_round_trip() {
    use opensynapse_core::ButtonAction as A;
    // Every offered kind maps to a profile action and back to the same edit.
    for kind in ActionKind::choices() {
        let e = ActionEdit {
            kind,
            usage: 0x2C,
            modifiers: 0x06,
        };
        match e.to_action() {
            Some(a) => {
                let back = ActionEdit::from_action(&a);
                assert_eq!(back.kind, kind, "{a:?}");
                if kind == ActionKind::Key {
                    assert_eq!(back, e);
                }
            }
            None => assert_eq!(kind, ActionKind::Other),
        }
    }
    assert_eq!(
        ActionEdit::new(ActionKind::Key).to_action(),
        Some(A::Key {
            key: "a".into(),
            mods: vec![]
        })
    );
    assert_eq!(
        ActionEdit::new(ActionKind::Mouse(4)).to_action(),
        Some(A::MouseButton { target: 4 })
    );
    // Actions the editor does not offer, and names the core mapping rejects.
    for a in [
        A::MouseButton { target: 7 },
        A::DoubleClick { target: 2 },
        A::Key {
            key: "nope".into(),
            mods: vec![],
        },
        A::Key {
            key: "a".into(),
            mods: vec!["hyper".into()],
        },
    ] {
        assert_eq!(ActionEdit::from_action(&a).kind, ActionKind::Other, "{a:?}");
    }
}

#[test]
fn primary_guard_names_the_button() {
    assert!(primary_guard(4, false).is_ok());
    assert!(primary_guard(1, true).is_ok());
    let e = primary_guard(2, false).unwrap_err();
    assert!(e.contains("Right click") && e.contains("allow editing left/right click"));
}

/// A registry device with extended (0x0F) lighting and zones wheel, logo, underglow.
fn rgb_mouse() -> &'static Device {
    Registry::embedded()
        .devices()
        .iter()
        .find(|d| {
            d.lighting_family() == Some(LightingFamily::Extended)
                && d.lighting_zones() == [1, 4, 10]
                && d.lighting_brightness()
        })
        .expect("an extended-lighting mouse in the registry")
}

const GREEN: lighting::Rgb = lighting::Rgb(0, 0xFF, 0);

/// Fake mouse whose logo (volatile storage) runs spectrum at full brightness.
fn lit_mouse() -> FakeMouse {
    let m = FakeMouse::new();
    let spectrum =
        lighting::Effect::from_mode(lighting::Storage::Volatile, 4, lighting::Mode::Spectrum);
    m.state().lighting.insert((0, 4), (spectrum, 255));
    m
}

#[test]
fn lighting_write_verifies_and_restores() {
    let m = lit_mouse();
    let mut s = session(&m);
    let new = lighting_setting(
        lighting::Storage::Volatile,
        4,
        lighting::Mode::Static(GREEN),
        Some(80),
    );
    let out = guarded_write(&mut s, new.clone(), Some(rgb_mouse()), None, NO_OPTS).unwrap();
    assert!(out.verified(), "{out:?}");
    assert_eq!(out.now, new);
    assert_eq!(
        out.old.to_string(),
        "lighting (logo): spectrum, brightness 100%"
    );
    assert_eq!(
        out.new.to_string(),
        "lighting (logo): static 00ff00, brightness 80%"
    );
    {
        let st = m.state();
        let (e, b) = &st.lighting[&(0, 4)];
        assert_eq!((e.mode(), *b), (Some(lighting::Mode::Static(GREEN)), 204));
    }
    // Effect first, then brightness.
    let sets: Vec<(u8, u8)> = m
        .sent()
        .into_iter()
        .filter(|(_, id)| id & 0x80 == 0)
        .collect();
    assert_eq!(sets, [(0x0F, 0x02), (0x0F, 0x04)]);
    let back = guarded_write(&mut s, out.old, Some(rgb_mouse()), None, RESTORE).unwrap();
    assert!(back.verified());
    assert_eq!(
        m.state().lighting[&(0, 4)].0.mode(),
        Some(lighting::Mode::Spectrum)
    );
}

#[test]
fn lighting_refused_without_extended_family() {
    let m = lit_mouse();
    let new = lighting_setting(lighting::Storage::Volatile, 4, lighting::Mode::Off, None);
    // The Viper is non-Chroma.
    let err =
        guarded_write(&mut session(&m), new.clone(), Some(viper()), None, NO_OPTS).unwrap_err();
    assert!(err.message.contains("no lighting"), "{}", err.message);
    // A zone the device does not have.
    let bad = lighting_setting(lighting::Storage::Volatile, 5, lighting::Mode::Off, None);
    assert!(guarded_write(&mut session(&m), bad, Some(rgb_mouse()), None, NO_OPTS).is_err());
    // Standard-family devices are CLI-only.
    let kb = Registry::embedded()
        .devices()
        .iter()
        .find(|d| d.lighting_family() == Some(LightingFamily::Standard))
        .unwrap();
    let err = validate(&new, Some(kb), NO_OPTS).unwrap_err();
    assert!(err.contains("CLI"), "{err}");
    assert_eq!(m.writes(), 0);
}

#[test]
fn lighting_unreadable_zone_is_never_written() {
    let m = lit_mouse();
    // Zone 1 (wheel) is valid for the device, but the fake does not answer it.
    let new = lighting_setting(lighting::Storage::Volatile, 1, lighting::Mode::Off, None);
    let err = guarded_write(&mut session(&m), new, Some(rgb_mouse()), None, NO_OPTS).unwrap_err();
    assert!(err.old.is_none());
    assert_eq!(m.writes(), 0);
}
