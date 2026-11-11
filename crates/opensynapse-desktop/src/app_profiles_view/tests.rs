use super::*;

/// A fresh, empty folder for one test.
fn tmp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "opensynapse-desktop-app-profiles-{}-{test}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn rule(exe: &str, profile: &str) -> AppRule {
    AppRule {
        exe: exe.into(),
        profile: profile.into(),
    }
}

#[test]
fn atomic_write_creates_and_replaces() {
    let dir = tmp_dir("atomic");
    let file = dir.join("sub").join("app-profiles.toml");
    write_atomic(&file, "a = 1\n").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "a = 1\n");
    write_atomic(&file, "b = 2\n").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "b = 2\n");
    // Only the target is left behind, no temporary file.
    let names: Vec<_> = fs::read_dir(dir.join("sub"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("app-profiles.toml")]);
    // A folder in the way: the write fails and leaves no temporary file.
    let blocked = dir.join("sub").join("blocked.toml");
    fs::create_dir_all(blocked.join("x")).unwrap();
    assert!(write_atomic(&blocked, "c = 3\n").is_err());
    assert_eq!(fs::read_dir(dir.join("sub")).unwrap().count(), 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn suggested_paths() {
    assert_eq!(suggested_profile_path("cs2.exe"), "profiles/cs2.toml");
    assert_eq!(
        suggested_profile_path(r"C:\Games\My Game.exe"),
        "profiles/MyGame.toml"
    );
    assert_eq!(suggested_profile_path("C:/x/a.b.exe"), "profiles/a.b.toml");
    assert_eq!(suggested_profile_path("  "), "profiles/profile.toml");
}

#[test]
fn rule_checks_are_inline_per_row() {
    let rules = AppProfiles {
        default: Some(" ".into()),
        apps: vec![
            rule("cs2.exe", "p.toml"),
            rule("", "p.toml"),
            rule("a.exe", ""),
            rule("CS2.exe", "q.toml"),
        ],
    };
    let issues = rule_issues(&rules);
    assert!(issues[0].is_empty());
    assert_eq!(issues[1], vec!["`exe` must not be empty".to_owned()]);
    assert_eq!(issues[2], vec!["`profile` must not be empty".to_owned()]);
    assert_eq!(issues[3].len(), 1);
    assert!(issues[3][0].contains("rule 1"), "{:?}", issues[3]);
    assert!(default_issue(&rules).is_some());
    assert!(default_issue(&AppProfiles::default()).is_none());
    assert!(exe_hint("cs2").is_some());
    assert!(exe_hint("cs2.exe").is_none());
    assert!(exe_hint("").is_none());
}

#[test]
fn form_round_trips_a_profile() {
    let text = "[device]\npid = \"00B8\"\n[dpi]\nstages = [800, 1600, 3200]\nactive_stage = 2\n\
                [polling]\nhz = 500\n[power]\nsleep_seconds = 120\n\
                [buttons.4]\naction = \"dpi-up\"\n[hypershift.buttons.5]\naction = \"key\"\nkey = \"a\"\n";
    let p = Profile::from_toml(text).unwrap();
    let f = ProfileForm::from_profile(&p);
    assert_eq!(f.pid, "00B8");
    assert_eq!(f.active, 1);
    assert_eq!(f.buttons.count(), 2);
    assert_eq!(f.to_profile(false).unwrap(), p);

    // Sections switched off are left out; buttons stay.
    let mut f = f;
    f.dpi_on = false;
    f.polling_on = false;
    let q = f.to_profile(false).unwrap();
    assert!(q.dpi.is_none() && q.polling.is_none());
    assert_eq!(q.power, p.power);
    assert_eq!(q.buttons, p.buttons);
}

#[test]
fn form_checks() {
    let errs = |f: &ProfileForm| f.to_profile(false).unwrap_err().join("\n");
    let mut f = ProfileForm::new(None);
    assert!(errs(&f).contains("device.pid: needed"));
    f.pid = "B8".into();
    assert!(errs(&f).contains("four hex digits"));
    f.pid = "00b8".into();
    assert!(errs(&f).contains("sets nothing"));
    f.polling_on = true;
    assert_eq!(
        f.to_profile(false).unwrap().device.unwrap().pid,
        Pid(0x00B8)
    );
    // Registry limits of the Viper (100-30000, step 50).
    f.dpi_on = true;
    f.stages = vec![125, 1600];
    f.active = 1;
    assert!(errs(&f).contains("dpi.stages[0]"));
    f.stages = vec![150, 1600];
    assert!(f.to_profile(false).is_ok());
    f.hz = 250;
    assert!(errs(&f).contains("polling.hz"));
    // Prefilled from the selected device.
    assert_eq!(ProfileForm::new(Some(0x00B8)).pid, "00B8");
}

#[test]
fn rules_file_prefers_the_agents() {
    let info = AppProfilesInfo {
        config: Some(r"D:\x\rules.toml".into()),
        ..AppProfilesInfo::default()
    };
    assert_eq!(
        rules_file(&AgentState::Info(info), Some(r"C:\AppData")),
        Some(PathBuf::from(r"D:\x\rules.toml"))
    );
    assert_eq!(
        rules_file(&AgentState::NotRunning, Some(r"C:\AppData")),
        Some(
            PathBuf::from(r"C:\AppData")
                .join("OpenSynapse")
                .join("app-profiles.toml")
        )
    );
    assert_eq!(rules_file(&AgentState::NotRunning, None), None);
}

#[test]
fn agent_messages() {
    assert!(
        reload_note(&AgentState::NotRunning)
            .unwrap()
            .contains("not running")
    );
    let ok = AppProfilesInfo {
        enabled: true,
        rules: vec![opensynapse_ipc::msg::AppRuleInfo {
            exe: "a.exe".into(),
            profile: "p".into(),
        }],
        foreground: Some(r"C:\x\Code.exe".into()),
        ..AppProfilesInfo::default()
    };
    assert!(
        reload_note(&AgentState::Info(ok.clone()))
            .unwrap()
            .contains("1 rule")
    );
    assert!(agent_line(&AgentState::Info(ok)).contains("app in front: Code.exe"));
    let bad = AppProfilesInfo {
        error: Some("rule 1: bad".into()),
        ..AppProfilesInfo::default()
    };
    assert!(
        reload_note(&AgentState::Info(bad))
            .unwrap_err()
            .contains("rejected")
    );
    assert!(reload_note(&AgentState::Error("x".into())).is_err());
}

fn view_with(rules: Vec<AppRule>, editing: usize) -> AppProfilesView {
    let mut v = AppProfilesView {
        file: Some(PathBuf::from(r"C:\nowhere\app-profiles.toml")),
        ..AppProfilesView::default()
    };
    v.rules.apps = rules;
    let form = ProfileForm::new(None);
    v.editor = Some(Editor {
        target: Target::Rule(editing),
        orig: form.clone(),
        form,
        loaded_from: PathBuf::new(),
        new_file: false,
        blocked: None,
        allow_primary: false,
    });
    v
}

fn target(v: &AppProfilesView) -> Option<Target> {
    v.editor.as_ref().map(|e| e.target)
}

#[test]
fn row_actions_keep_the_editor_on_its_rule() {
    let three = || vec![rule("a.exe", "a"), rule("b.exe", "b"), rule("c.exe", "c")];
    let mut v = view_with(three(), 1);
    v.apply(RowAction::Up(1), None);
    assert_eq!(v.rules.apps[0].exe, "b.exe");
    assert_eq!(target(&v), Some(Target::Rule(0)));
    v.apply(RowAction::Down(0), None);
    assert_eq!(target(&v), Some(Target::Rule(1)));
    v.apply(RowAction::Remove(0), None);
    assert_eq!(target(&v), Some(Target::Rule(0)));
    assert_eq!(v.rules.apps[0].exe, "b.exe");
    v.apply(RowAction::Remove(0), None);
    assert_eq!(target(&v), None);
    v.apply(RowAction::Add, None);
    assert_eq!(v.rules.apps.len(), 2);
    assert_eq!(v.rules.apps[1], rule("", ""));
}

#[test]
fn save_writes_profile_and_rules() {
    let dir = tmp_dir("save");
    let file = dir.join("app-profiles.toml");
    let mut v = view_with(vec![rule("cs2.exe", "profiles/cs2.toml")], 0);
    v.file = Some(file.clone());
    let ed = v.editor.as_mut().unwrap();
    ed.new_file = true;
    ed.form.pid = "00B8".into();
    ed.form.polling_on = true;
    ed.form.hz = 500;
    assert!(v.dirty());
    assert!(v.save_blocker(&rule_issues(&v.rules), None).is_none());
    let msg = v.write_files().unwrap();
    assert!(msg.starts_with("Saved"), "{msg}");
    assert!(!msg.contains("not found"), "{msg}");
    assert!(!v.dirty());

    let rules = AppProfiles::from_toml(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(rules.apps, vec![rule("cs2.exe", "profiles/cs2.toml")]);
    let p = Profile::from_toml(&fs::read_to_string(dir.join("profiles").join("cs2.toml")).unwrap())
        .unwrap();
    assert_eq!(p.polling, Some(PollingSection { hz: 500 }));
    assert_eq!(p.device.unwrap().pid, Pid(0x00B8));

    // A rule whose profile file is missing is saved, with a note.
    v.rules.apps.push(rule("b.exe", "profiles/missing.toml"));
    let msg = v.write_files().unwrap();
    assert!(msg.contains("not found: profiles/missing.toml"), "{msg}");

    // Invalid profile settings block the save.
    v.editor.as_mut().unwrap().form.hz = 250;
    assert!(v.save_blocker(&rule_issues(&v.rules), None).is_some());
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Buttons

use buttons::Slot;
use opensynapse_core::{ButtonAction, ButtonId};
use opensynapse_device::{ActionEdit, ActionKind, VIPER_BUTTONS};
use opensynapse_protocol::buttons::Mode;

const WITH_BUTTONS: &str = "[device]\npid = \"00B8\"\n\
    [buttons.4]\naction = \"dpi-down\"\n\
    [buttons.7]\naction = \"mouse-button\"\ntarget = 7\n\
    [hypershift.buttons.5]\naction = \"key\"\nkey = \"a\"\nmods = [\"ctrl\"]\n";

fn with_buttons() -> ProfileForm {
    ProfileForm::from_profile(&Profile::from_toml(WITH_BUTTONS).unwrap())
}

#[test]
fn buttons_load_into_rows() {
    let f = with_buttons();
    let b = &f.buttons;
    // Without a device list: the Viper layout, then IDs only the profile has.
    let mut want = VIPER_BUTTONS.to_vec();
    want.push(7);
    assert_eq!(b.row_ids(&[]), want);
    // With the selected device's list.
    assert_eq!(b.row_ids(&[1, 2, 5]), vec![1, 2, 5, 4, 7]);
    assert_eq!(
        b.slot(Mode::Normal, 4),
        Slot::Set(ActionEdit::new(ActionKind::DpiDown))
    );
    assert_eq!(b.slot(Mode::Normal, 1), Slot::Unchanged);
    assert_eq!(b.slot(Mode::HyperShift, 4), Slot::Unchanged);
    // Mouse button 7 is not an editor choice: shown read-only, kept as read.
    assert_eq!(
        b.slot(Mode::Normal, 7),
        Slot::Kept(ButtonAction::MouseButton { target: 7 })
    );
    let Slot::Set(key) = b.slot(Mode::HyperShift, 5) else {
        panic!("key slot");
    };
    assert_eq!(key.kind, ActionKind::Key);
    assert_eq!((key.usage, key.modifiers), (0x04, 0x01));
    assert_eq!(b.count(), 3);
    // Unedited: the file comes back as it was.
    assert_eq!(
        f.to_profile(false).unwrap(),
        Profile::from_toml(WITH_BUTTONS).unwrap()
    );
}

#[test]
fn button_edits_round_trip_through_toml() {
    let mut f = with_buttons();
    let orig = f.clone();
    let f1 = ActionEdit {
        kind: ActionKind::Key,
        usage: 0x3A,
        modifiers: 0x02 | 0x04,
    };
    f.buttons.set(Mode::Normal, 5, Slot::Set(f1));
    f.buttons.set(
        Mode::HyperShift,
        96,
        Slot::Set(ActionEdit::new(ActionKind::Off)),
    );
    f.buttons.set(
        Mode::Normal,
        4,
        Slot::Set(ActionEdit::new(ActionKind::DoubleClick)),
    );
    assert_ne!(f, orig);
    let p = f.to_profile(false).unwrap();
    let text = p.to_toml().unwrap();
    let back = Profile::from_toml(&text).unwrap();
    assert_eq!(back, p);
    assert_eq!(
        back.buttons[&ButtonId(5)],
        ButtonAction::Key {
            key: "f1".into(),
            mods: vec!["shift".into(), "alt".into()]
        }
    );
    assert_eq!(
        back.buttons[&ButtonId(4)],
        ButtonAction::DoubleClick { target: 1 }
    );
    assert_eq!(
        back.hypershift.as_ref().unwrap().buttons[&ButtonId(96)],
        ButtonAction::Off {}
    );
    // Loading the written file gives the same form.
    assert_eq!(ProfileForm::from_profile(&back), f);

    // Picking another action and then the old one again is not an edit.
    let mut g = orig.clone();
    let Slot::Set(e) = g.buttons.slot(Mode::Normal, 4) else {
        panic!()
    };
    g.buttons.set(
        Mode::Normal,
        4,
        Slot::Set(ActionEdit {
            kind: ActionKind::Key,
            ..e
        }),
    );
    g.buttons.set(Mode::Normal, 4, Slot::Set(e));
    assert_eq!(g, orig);
}

#[test]
fn unchanged_removes_the_key() {
    let mut f = with_buttons();
    f.buttons.set(Mode::Normal, 4, Slot::Unchanged);
    f.buttons.set(Mode::HyperShift, 5, Slot::Unchanged);
    let p = f.to_profile(false).unwrap();
    assert!(!p.buttons.contains_key(&ButtonId(4)));
    assert!(p.buttons.contains_key(&ButtonId(7)));
    // An empty HyperShift map leaves the section out.
    assert_eq!(p.hypershift, None);
    let text = p.to_toml().unwrap();
    assert!(
        !text.contains("buttons.4") && !text.contains("hypershift"),
        "{text}"
    );
    // The row stays (Viper layout) and shows "unchanged".
    assert!(f.buttons.row_ids(&[]).contains(&4));
    assert_eq!(f.buttons.slot(Mode::Normal, 4), Slot::Unchanged);
    // Nothing left at all: the profile sets nothing.
    f.buttons.set(Mode::Normal, 7, Slot::Unchanged);
    assert!(
        f.to_profile(false)
            .unwrap_err()
            .join("\n")
            .contains("sets nothing")
    );
}

#[test]
fn left_and_right_click_need_the_opt_in() {
    let mut f = with_buttons();
    f.buttons.set(
        Mode::Normal,
        1,
        Slot::Set(ActionEdit::new(ActionKind::Mouse(3))),
    );
    let errs = f.to_profile(false).unwrap_err().join("\n");
    assert!(
        errs.contains("buttons.1: Left click is protected"),
        "{errs}"
    );
    assert!(f.to_profile(true).is_ok());
    // Also in HyperShift mode, and for "unchanged" -> action.
    let mut f = with_buttons();
    f.buttons.set(
        Mode::HyperShift,
        2,
        Slot::Set(ActionEdit::new(ActionKind::Off)),
    );
    let errs = f.to_profile(false).unwrap_err().join("\n");
    assert!(errs.contains("hypershift.buttons.2: Right click"), "{errs}");

    // A file that already sets left click loads and saves without the opt-in;
    // changing or removing that entry needs it.
    let text = "[device]\npid = \"00B8\"\n[buttons.1]\naction = \"mouse-button\"\ntarget = 1\n";
    let mut f = ProfileForm::from_profile(&Profile::from_toml(text).unwrap());
    assert!(f.to_profile(false).is_ok());
    f.buttons.set(Mode::Normal, 1, Slot::Unchanged);
    f.polling_on = true;
    assert!(f.to_profile(false).is_err());
    assert!(f.to_profile(true).is_ok());

    // The editor's Save check uses the editor's opt-in.
    let mut v = view_with(vec![rule("a.exe", "p.toml")], 0);
    let ed = v.editor.as_mut().unwrap();
    ed.form = with_buttons();
    ed.form.buttons.set(
        Mode::Normal,
        2,
        Slot::Set(ActionEdit::new(ActionKind::Mouse(1))),
    );
    assert!(v.save_blocker(&rule_issues(&v.rules), None).is_some());
    v.editor.as_mut().unwrap().allow_primary = true;
    assert!(v.save_blocker(&rule_issues(&v.rules), None).is_none());
}
