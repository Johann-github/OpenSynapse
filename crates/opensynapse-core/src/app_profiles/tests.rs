use std::path::{Path, PathBuf};

use super::{exe_matches, resolve_profile_path};
use crate::{AppProfiles, AppProfilesError, AppRule, Profile};

const RULES: &str = r#"
# profile paths relative to this file's folder or absolute
default = "profiles/desktop.toml"
[[app]]
exe = "cs2.exe"
profile = "profiles/cs2.toml"
[[app]]
exe = 'C:\Games\Other\game.exe'
profile = 'D:\p\game.toml'
"#;

#[test]
fn parses_rules_and_default() {
    let p = AppProfiles::from_toml(RULES).unwrap();
    assert_eq!(p.default.as_deref(), Some("profiles/desktop.toml"));
    assert_eq!(
        p.apps[0],
        AppRule {
            exe: "cs2.exe".into(),
            profile: "profiles/cs2.toml".into()
        }
    );
    assert_eq!(p.apps.len(), 2);
}

#[test]
fn empty_file_has_no_rules() {
    assert_eq!(AppProfiles::from_toml("").unwrap(), AppProfiles::default());
}

#[test]
fn rejects_bad_files() {
    assert!(matches!(
        AppProfiles::from_toml("[[app]]\nexe = \"a.exe\"\n"),
        Err(AppProfilesError::Parse(_))
    ));
    assert!(matches!(
        AppProfiles::from_toml("bogus = 1\n"),
        Err(AppProfilesError::Parse(_))
    ));
    assert!(matches!(
        AppProfiles::from_toml("[[app]]\nexe = \"a.exe\"\nprofile = \"p\"\nextra = 1\n"),
        Err(AppProfilesError::Parse(_))
    ));
    let e = AppProfiles::from_toml(
        "[[app]]\nexe = \"a.exe\"\nprofile = \"p\"\n[[app]]\nexe = \" \"\nprofile = \"p\"\n",
    )
    .unwrap_err();
    assert_eq!(
        e.to_string(),
        "app-profiles rule 2: `exe` must not be empty"
    );
    assert!(matches!(
        AppProfiles::from_toml("[[app]]\nexe = \"a.exe\"\nprofile = \"\"\n"),
        Err(AppProfilesError::Rule { index: 1, .. })
    ));
    assert!(matches!(
        AppProfiles::from_toml("default = \"\"\n"),
        Err(AppProfilesError::EmptyDefault)
    ));
}

#[test]
fn matching_by_name_and_path() {
    assert!(exe_matches("cs2.exe", r"C:\Steam\cs2.exe"));
    assert!(exe_matches("CS2.EXE", r"c:\steam\Cs2.exe"));
    assert!(!exe_matches("cs2.exe", r"C:\Steam\xcs2.exe"));
    assert!(!exe_matches("cs2", r"C:\Steam\cs2.exe"));
    assert!(exe_matches("notepad.exe", "notepad.exe"));
    assert!(exe_matches(r"C:\Games\game.exe", r"c:\games\GAME.exe"));
    assert!(exe_matches("C:/Games/game.exe", r"C:\Games\game.exe"));
    assert!(!exe_matches(r"C:\Games\game.exe", r"D:\Games\game.exe"));
}

#[test]
fn profile_for_uses_first_rule_then_default() {
    let mut p = AppProfiles::from_toml(RULES).unwrap();
    assert_eq!(p.profile_for(r"C:\x\cs2.exe"), Some("profiles/cs2.toml"));
    assert_eq!(
        p.profile_for(r"C:\Games\Other\game.exe"),
        Some(r"D:\p\game.toml")
    );
    assert_eq!(
        p.profile_for(r"C:\x\game.exe"),
        Some("profiles/desktop.toml")
    );
    p.default = None;
    assert_eq!(p.profile_for(r"C:\x\game.exe"), None);
    p.apps.insert(
        0,
        AppRule {
            exe: "cs2.exe".into(),
            profile: "first.toml".into(),
        },
    );
    assert_eq!(p.profile_for(r"C:\x\cs2.exe"), Some("first.toml"));
}

#[test]
fn profile_paths_resolve_against_the_rules_folder() {
    let rules = Path::new(r"C:\Users\u\AppData\Roaming\OpenSynapse\app-profiles.toml");
    #[cfg(windows)]
    {
        assert_eq!(
            resolve_profile_path(rules, "profiles/cs2.toml"),
            PathBuf::from(r"C:\Users\u\AppData\Roaming\OpenSynapse").join("profiles/cs2.toml")
        );
        assert_eq!(
            resolve_profile_path(rules, r"D:\p\x.toml"),
            PathBuf::from(r"D:\p\x.toml")
        );
    }
    #[cfg(unix)]
    {
        let _ = rules;
        assert_eq!(
            resolve_profile_path(Path::new("/c/app-profiles.toml"), "/p/x.toml"),
            PathBuf::from("/p/x.toml")
        );
        assert_eq!(
            resolve_profile_path(Path::new("/c/app-profiles.toml"), "p/x.toml"),
            PathBuf::from("/c/p/x.toml")
        );
    }
    assert_eq!(
        resolve_profile_path(Path::new("app-profiles.toml"), "x.toml"),
        PathBuf::from("x.toml")
    );
}

#[test]
fn fill_missing_keeps_first_values() {
    let mut first = Profile::from_toml(
        "[device]\npid = \"00B8\"\n[polling]\nhz = 1000\n\
         [buttons.4]\naction = \"mouse-button\"\ntarget = 4\n",
    )
    .unwrap();
    let later = Profile::from_toml(
        "[device]\npid = \"00B8\"\n[polling]\nhz = 500\n[power]\nsleep_seconds = 60\n\
         [buttons.4]\naction = \"dpi-up\"\n[buttons.5]\naction = \"dpi-up\"\n\
         [hypershift.buttons.4]\naction = \"dpi-up\"\n",
    )
    .unwrap();
    first.fill_missing_from(&later);
    let want = Profile::from_toml(
        "[device]\npid = \"00B8\"\n[polling]\nhz = 1000\n[power]\nsleep_seconds = 60\n\
         [buttons.4]\naction = \"mouse-button\"\ntarget = 4\n[buttons.5]\naction = \"dpi-up\"\n\
         [hypershift.buttons.4]\naction = \"dpi-up\"\n",
    )
    .unwrap();
    assert_eq!(first, want);
}
