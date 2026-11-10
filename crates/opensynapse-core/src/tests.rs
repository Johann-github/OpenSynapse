use opensynapse_protocol::{
    buttons::{self, Mode},
    dpi::{self, Dpi, DpiStage, DpiStages, Storage},
    polling, power,
};

use crate::*;

const EXAMPLE: &str = r#"
[device]
pid = "00B8"
[dpi]
stages = [400, 800, 1600, 3200, 6400]
active_stage = 2
[polling]
hz = 1000
[power]
sleep_seconds = 300
[buttons.4]          # normal mode
action = "key"
key = "a"
mods = ["ctrl"]
[buttons.5]
action = "mouse-button"
target = 5
[hypershift.buttons.4]
action = "dpi-up"
"#;

/// Viper V3 HyperSpeed range from `data/devices/mice.toml`.
fn viper() -> DeviceLimits {
    DeviceLimits {
        dpi_min: 100,
        dpi_max: 30000,
        dpi_step: 50,
        max_dpi_stages: 5,
    }
}

fn example() -> Profile {
    Profile::from_toml(EXAMPLE).unwrap()
}

#[test]
fn example_parses() {
    let p = example();
    assert_eq!(p.device.unwrap().pid, Pid(0x00B8));
    let d = p.dpi.unwrap();
    assert_eq!(d.stages, [400, 800, 1600, 3200, 6400]);
    assert_eq!(d.active_stage, 2);
    assert_eq!(p.polling.unwrap().hz, 1000);
    assert_eq!(p.power.unwrap().sleep_seconds, 300);
    assert_eq!(
        p.buttons[&ButtonId(4)],
        ButtonAction::Key {
            key: "a".into(),
            mods: vec!["ctrl".into()]
        }
    );
    assert_eq!(
        p.buttons[&ButtonId(5)],
        ButtonAction::MouseButton { target: 5 }
    );
    assert_eq!(
        p.hypershift.unwrap().buttons[&ButtonId(4)],
        ButtonAction::DpiUp {}
    );
    assert_eq!(example().validate(&viper()), Ok(()));
}

#[test]
fn round_trip() {
    let p = example();
    let text = p.to_toml().unwrap();
    assert_eq!(Profile::from_toml(&text).unwrap(), p);
    // Stable: serializing the reparsed profile gives the same text.
    assert_eq!(Profile::from_toml(&text).unwrap().to_toml().unwrap(), text);
    assert!(text.contains("pid = \"00B8\""), "{text}");
    assert!(text.contains("[buttons.4]"), "{text}");
    assert!(text.contains("[hypershift.buttons.4]"), "{text}");
    assert!(text.contains("action = \"dpi-up\""), "{text}");
}

#[test]
fn every_section_optional() {
    let p = Profile::from_toml("").unwrap();
    assert_eq!(p, Profile::default());
    assert_eq!(p.to_toml().unwrap(), "");
    assert_eq!(p.plan(&Target::default()).unwrap(), vec![]);

    let only = Profile::from_toml("[polling]\nhz = 500\n").unwrap();
    assert!(only.dpi.is_none() && only.buttons.is_empty() && only.hypershift.is_none());
    assert_eq!(only.plan(&Target::default()).unwrap().len(), 1);
}

#[test]
fn shape_errors_are_parse_errors() {
    for bad in [
        "[dpi]\nstages = [400]\n",                        // missing active_stage
        "[buttons.4]\naction = \"key\"\n",                // key action without key
        "[buttons.4]\naction = \"teleport\"\n",           // unknown action
        "[buttons.4]\naction = \"dpi-up\"\ntarget = 1\n", // stray field
        "[buttons.x]\naction = \"dpi-up\"\n",             // non-numeric button
        "[device]\npid = \"B8\"\n",                       // short PID
        "[device]\npid = \"00B8\"\nname = \"viper\"\n",   // unknown field
        "[colour]\nr = 1\n",                              // unknown section
        "[dpi]\nstages = [70000]\nactive_stage = 1\n",    // u16 overflow
    ] {
        let e = Profile::from_toml(bad).unwrap_err();
        assert!(matches!(e, TomlError::Parse(_)), "{bad}");
    }
    let e = Profile::from_toml("[buttons.4]\naction = \"key\"\n").unwrap_err();
    assert!(e.to_string().contains("key"), "{e}");
}

#[test]
fn pid_forms() {
    let p = Profile::from_toml("[device]\npid = \"0x00b8\"\n").unwrap();
    assert_eq!(p.device.unwrap().pid, Pid(0xB8));
    assert_eq!(Pid(0xB8).to_string(), "00B8");
}

fn issues(p: &Profile) -> Vec<String> {
    p.validate(&viper())
        .unwrap_err()
        .0
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn validation_errors() {
    let p = Profile::from_toml(
        r#"
[dpi]
stages = [50, 825, 1600]
active_stage = 4
[polling]
hz = 250
[buttons.0]
action = "mouse-button"
target = 9
[buttons.4]
action = "key"
key = "pgup"
mods = ["ctrl", "hyper", "CTRL"]
[hypershift.buttons.5]
action = "double-click"
target = 0
"#,
    )
    .unwrap();
    assert_eq!(
        issues(&p),
        [
            "dpi.stages[0]: 50 DPI is outside 100-30000 (step 50)",
            "dpi.stages[1]: 825 DPI is outside 100-30000 (step 50)",
            "dpi.active_stage: must be 1 to 3 (1-based), is 4",
            "polling.hz: 250 Hz is not one of [125, 500, 1000]",
            "buttons.0: button ID must be 1 or higher",
            "buttons.0.target: mouse button must be 1 to 8, is 9",
            "buttons.4.key: unknown key \"pgup\" (a-z, 0-9, f1-f12, enter, esc, backspace, tab, space)",
            "buttons.4.mods: unknown modifier \"hyper\" (ctrl, shift, alt, win)",
            "buttons.4.mods: modifier \"CTRL\" repeated",
            "hypershift.buttons.5.target: mouse button must be 1 to 8, is 0",
        ]
    );
    let e = p.validate(&viper()).unwrap_err();
    assert!(
        e.to_string()
            .starts_with("profile has 10 problem(s)\n  dpi.stages[0]")
    );
    // plan refuses an invalid profile.
    assert_eq!(p.plan(&Target::default()).unwrap_err().0.len(), 8);
}

#[test]
fn stage_count_limits() {
    let empty = Profile::from_toml("[dpi]\nstages = []\nactive_stage = 1\n").unwrap();
    assert_eq!(issues(&empty), ["dpi.stages: needs 1 to 5 stages, has 0"]);

    let six = Profile::from_toml(
        "[dpi]\nstages = [400, 800, 1200, 1600, 2000, 2400]\nactive_stage = 1\n",
    )
    .unwrap();
    assert_eq!(issues(&six), ["dpi.stages: needs 1 to 5 stages, has 6"]);
    // Without device limits the protocol maximum (11) applies.
    assert_eq!(six.validate(&DeviceLimits::default()), Ok(()));
    let mut twelve = six.clone();
    twelve.dpi.as_mut().unwrap().stages = vec![800; 12];
    assert!(twelve.validate(&DeviceLimits::default()).is_err());
}

#[test]
fn plan_matches_protocol_builders() {
    let target = Target {
        limits: viper(),
        ..Target::default()
    };
    let plan = example().plan(&target).unwrap();

    let stages = DpiStages {
        active: 2,
        stages: [400, 800, 1600, 3200, 6400]
            .iter()
            .zip(1u8..)
            .map(|(&d, id)| DpiStage {
                id,
                dpi: Dpi::uniform(d),
            })
            .collect(),
    };
    let want = vec![
        PlannedWrite {
            setting: Setting::DpiStages,
            request: dpi::set_dpi_stages(Storage::Persistent, &stages).unwrap(),
        },
        PlannedWrite {
            setting: Setting::PollingRate,
            request: polling::set_polling_rate(1000).unwrap(),
        },
        PlannedWrite {
            setting: Setting::SleepTime,
            request: power::set_sleep_time(300),
        },
        PlannedWrite {
            setting: Setting::Button {
                mode: Mode::Normal,
                button: 4,
            },
            request: buttons::set_button_assignment(&buttons::keyboard(
                1,
                4,
                Mode::Normal,
                0x01,
                0x04,
            ))
            .unwrap(),
        },
        PlannedWrite {
            setting: Setting::Button {
                mode: Mode::Normal,
                button: 5,
            },
            request: buttons::set_button_assignment(&buttons::mouse_button(1, 5, Mode::Normal, 5))
                .unwrap(),
        },
        PlannedWrite {
            setting: Setting::Button {
                mode: Mode::HyperShift,
                button: 4,
            },
            request: buttons::set_button_assignment(&buttons::dpi_up(1, 4, Mode::HyperShift))
                .unwrap(),
        },
    ];
    assert_eq!(plan, want);
    assert_eq!(plan[5].setting.to_string(), "button 4 (HyperShift mode)");
}

#[test]
fn plan_honours_target() {
    let p = Profile::from_toml(
        "[dpi]\nstages = [800]\nactive_stage = 1\n[polling]\nhz = 500\n\
         [buttons.3]\naction = \"double-click\"\ntarget = 1\n",
    )
    .unwrap();
    let target = Target {
        onboard_profile: 2,
        dpi_storage: Storage::Volatile,
        polling: PollingVariant::Hyper,
        ..Target::default()
    };
    let plan = p.plan(&target).unwrap();
    let reqs: Vec<_> = plan.into_iter().map(|w| w.request).collect();
    assert_eq!(
        reqs,
        [
            dpi::set_dpi_stages(
                Storage::Volatile,
                &DpiStages {
                    active: 1,
                    stages: vec![DpiStage {
                        id: 1,
                        dpi: Dpi::uniform(800)
                    }],
                },
            )
            .unwrap(),
            polling::set_hyper_polling(2, 500).unwrap(),
            buttons::set_button_assignment(&buttons::double_click(2, 3, Mode::Normal, 1)).unwrap(),
        ]
    );
}

#[test]
fn key_with_several_mods() {
    let p = Profile::from_toml(
        "[buttons.4]\naction = \"key\"\nkey = \"F5\"\nmods = [\"shift\", \"alt\"]\n",
    )
    .unwrap();
    let plan = p.plan(&Target::default()).unwrap();
    assert_eq!(
        plan[0].request,
        buttons::set_button_assignment(&buttons::keyboard(1, 4, Mode::Normal, 0x06, 0x3E)).unwrap()
    );
    // No mods: field omitted on output.
    let bare = Profile::from_toml("[buttons.4]\naction = \"key\"\nkey = \"a\"\n").unwrap();
    assert!(!bare.to_toml().unwrap().contains("mods"));
}
