//! Fixed per-user install location and the pure plan of `opensynapse install` /
//! `uninstall` (`vault/Decisions/ADR-0009 Install location.md`).
//!
//! Windows: `%LOCALAPPDATA%\Programs\OpenSynapse` (per user, no admin rights, the
//! folder Windows itself suggests for per-user programs). Linux: `~/.local/bin`
//! (on `PATH` in common distributions; shared with other programs, so uninstall
//! keeps the folder; `vault/Decisions/ADR-0011 Linux autostart and install.md`).
//! Other systems (macOS, not decided yet): `$XDG_DATA_HOME/OpenSynapse/bin` or
//! `~/.local/share/OpenSynapse/bin`.
//!
//! Everything here except [`dir`] and [`copy_file`] is pure: the CLI asks which
//! files to copy or delete and what to do with autostart, then does the I/O.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::{AGENT_EXE, CLI_EXE, GUI_EXE, autostart};

/// Folder name under the per-user programs root.
pub const DIR_NAME: &str = "OpenSynapse";

/// Suffix of the temp copy written before the rename.
pub const TMP_SUFFIX: &str = ".tmp";
/// Suffix of a replaced binary that was still running (renamed aside, deleted
/// by the next install or uninstall).
pub const OLD_SUFFIX: &str = ".old";

/// One shipped binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binary {
    pub name: &'static str,
    /// `false`: install skips it with a note when the build has none.
    pub required: bool,
}

/// What an installation contains, in copy order (CLI, agent, GUI).
pub const BINARIES: [Binary; 3] = [
    Binary {
        name: CLI_EXE,
        required: true,
    },
    Binary {
        name: AGENT_EXE,
        required: true,
    },
    Binary {
        name: GUI_EXE,
        required: false,
    },
];

/// `<root>\Programs\OpenSynapse` for a root such as the `LOCALAPPDATA` value;
/// `None` when the root is missing or blank.
pub fn dir_from_local_appdata(root: Option<&str>) -> Option<PathBuf> {
    let root = root.filter(|d| !d.trim().is_empty())?;
    Some(PathBuf::from(root).join("Programs").join(DIR_NAME))
}

/// `$XDG_DATA_HOME/OpenSynapse/bin`, else `$HOME/.local/share/OpenSynapse/bin`.
pub fn dir_from_xdg(data_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let nonblank = |s: Option<&str>| s.filter(|d| !d.trim().is_empty()).map(PathBuf::from);
    let base =
        nonblank(data_home).or_else(|| Some(nonblank(home)?.join(".local").join("share")))?;
    Some(base.join(DIR_NAME).join("bin"))
}

/// `$HOME/.local/bin` (Linux: the per-user executables folder of the
/// file-hierarchy convention, on `PATH` in common distributions).
pub fn dir_from_home(home: Option<&str>) -> Option<PathBuf> {
    let home = home.filter(|h| !h.trim().is_empty())?;
    Some(PathBuf::from(home).join(".local").join("bin"))
}

/// The default install folder is shared with other programs (Linux
/// `~/.local/bin`): uninstall deletes only our files and keeps the folder.
pub const DEFAULT_DIR_SHARED: bool = cfg!(target_os = "linux");

/// The environment variable(s) the default folder comes from, for messages.
pub const DIR_ROOT_VAR: &str = if cfg!(windows) {
    "LOCALAPPDATA"
} else {
    "HOME"
};

/// The current user's install folder, if the environment names a root.
pub fn dir() -> Option<PathBuf> {
    let var = |k: &str| std::env::var(k).ok();
    if cfg!(windows) {
        dir_from_local_appdata(var("LOCALAPPDATA").as_deref())
    } else if cfg!(target_os = "linux") {
        dir_from_home(var("HOME").as_deref())
    } else {
        dir_from_xdg(var("XDG_DATA_HOME").as_deref(), var("HOME").as_deref())
    }
}

/// Same path? Compared component-wise; case-insensitive on Windows.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| {
                let s = c.as_os_str().to_string_lossy().into_owned();
                if cfg!(windows) { s.to_lowercase() } else { s }
            })
            .collect()
    };
    norm(a) == norm(b)
}

/// Is `file` directly inside `dir`?
pub fn is_in_dir(file: &Path, dir: &Path) -> bool {
    file.parent().is_some_and(|p| same_path(p, dir))
}

/// Files to copy into the install folder.
#[derive(Debug, PartialEq, Eq)]
pub struct CopyPlan {
    /// `(source, destination)` per binary found in the source folder.
    pub copies: Vec<(PathBuf, PathBuf)>,
    /// Optional binaries the source folder lacks.
    pub skipped: Vec<&'static str>,
}

/// Which binaries go from `src` (the folder of the running CLI) to `dst`.
/// Errors when both folders are the same or a required binary is missing.
pub fn plan_copy(
    src: &Path,
    dst: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Result<CopyPlan, String> {
    if same_path(src, dst) {
        return Err(format!(
            "this copy already runs from {}; run `install` from a new build \
             (e.g. target\\release\\{CLI_EXE} install)",
            dst.display()
        ));
    }
    let mut plan = CopyPlan {
        copies: Vec::new(),
        skipped: Vec::new(),
    };
    for b in BINARIES {
        let from = src.join(b.name);
        if exists(&from) {
            plan.copies.push((from, dst.join(b.name)));
        } else if b.required {
            return Err(format!(
                "{} not found; build it first ({BUILD_ALL})",
                from.display()
            ));
        } else {
            plan.skipped.push(b.name);
        }
    }
    Ok(plan)
}

/// Builds every shipped binary. A plain `cargo build --release` builds only the
/// CLI (`default-members`), leaving an old agent and GUI in `target\release`.
pub const BUILD_ALL: &str = "cargo build --release --workspace";

/// Slack before a binary counts as older than the CLI it is installed with (one
/// build links its binaries a few seconds apart).
const STALE_AFTER: Duration = Duration::from_secs(120);

/// A note when `name` (built at `built`) is clearly older than the CLI doing the
/// install (`cli_built`): most likely only the CLI was rebuilt, so install would
/// copy an old agent or GUI. A binary whose crate did not change is also older;
/// hence a note, not an error.
pub fn stale_note(name: &str, built: SystemTime, cli_built: SystemTime) -> Option<String> {
    let age = cli_built.duration_since(built).ok()?;
    (age > STALE_AFTER).then(|| {
        format!(
            "Note: {name} is {} min older than {CLI_EXE}. If it should contain newer \
             changes, build all binaries ({BUILD_ALL}) and install again.",
            age.as_secs() / 60
        )
    })
}

/// What install does with the login autostart.
#[derive(Debug, PartialEq, Eq)]
pub enum AutostartPlan {
    /// Write the Run value for the installed agent (new or replacing `previous`).
    Set { previous: Option<String> },
    /// Already points at the installed agent.
    Unchanged,
    /// Leave it alone (`--no-autostart`); the stored command, if any.
    Leave { current: Option<String> },
}

/// Autostart decision for install. Without `--no-autostart` the Run value is
/// pointed at `installed_agent`. With it, an existing entry is still moved over
/// when it starts the build being installed from (`source_dir`), so autostart
/// keeps working after that build is rebuilt or deleted; any other entry and an
/// absent one stay as they are.
pub fn plan_autostart(
    current: Option<&str>,
    installed_agent: &Path,
    source_dir: &Path,
    no_autostart: bool,
) -> AutostartPlan {
    match current {
        Some(cmd) if autostart::points_to(cmd, installed_agent) => AutostartPlan::Unchanged,
        Some(cmd) if !no_autostart || points_into(cmd, source_dir) => AutostartPlan::Set {
            previous: Some(cmd.to_owned()),
        },
        None if !no_autostart => AutostartPlan::Set { previous: None },
        _ => AutostartPlan::Leave {
            current: current.map(str::to_owned),
        },
    }
}

/// Does the Run command start a binary directly inside `dir`?
pub fn points_into(command: &str, dir: &Path) -> bool {
    is_in_dir(&autostart::exe_of(command), dir)
}

/// Start the installed agent after copying? When one ran before (it was
/// stopped for the copy) or autostart now starts it at login anyway.
pub fn start_after_install(was_running: bool, autostart_on: bool) -> bool {
    was_running || autostart_on
}

/// Every file uninstall may delete: the binaries plus leftover temp and
/// replaced copies, all directly inside `dir`. Nothing else is ever touched.
pub fn uninstall_files(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for b in BINARIES {
        v.push(dir.join(b.name));
        for suffix in [TMP_SUFFIX, OLD_SUFFIX] {
            v.push(dir.join(format!("{}{suffix}", b.name)));
        }
    }
    v
}

/// Why uninstall must not run from `current_exe`: a running exe inside the
/// install folder cannot delete itself.
pub fn uninstall_refusal(current_exe: &Path, dir: &Path) -> Option<String> {
    is_in_dir(current_exe, dir).then(|| {
        format!(
            "this copy runs from {}; run uninstall from another copy \
             (e.g. target\\release\\{CLI_EXE} uninstall)",
            dir.display()
        )
    })
}

/// Leftovers of an earlier install (`.old` / `.tmp`) to clean up before copying.
pub fn leftovers(dir: &Path) -> Vec<PathBuf> {
    uninstall_files(dir)
        .into_iter()
        .filter(|p| {
            let s = p.to_string_lossy();
            s.ends_with(TMP_SUFFIX) || s.ends_with(OLD_SUFFIX)
        })
        .collect()
}

/// `<path><suffix>` next to `path`.
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// How [`copy_file`] replaced the destination.
#[derive(Debug, PartialEq, Eq)]
pub enum Replaced {
    /// No file there before.
    New,
    /// The old file was replaced.
    Replaced,
    /// The old file was in use (a running GUI); it was renamed to `<name>.old`
    /// and is deleted by the next install or uninstall.
    MovedAside,
}

/// Copy `src` to `dst` through `<dst>.tmp` and a rename, so `dst` is never half
/// written. A destination that cannot be replaced because it is running is
/// renamed to `<dst>.old` first (Windows allows renaming a running exe).
pub fn copy_file(src: &Path, dst: &Path) -> io::Result<Replaced> {
    let ctx = |what: &str, p: &Path, e: io::Error| {
        io::Error::new(e.kind(), format!("cannot {what} {}: {e}", p.display()))
    };
    let tmp = with_suffix(dst, TMP_SUFFIX);
    std::fs::copy(src, &tmp).map_err(|e| ctx("write", &tmp, e))?;
    let existed = dst.exists();
    match std::fs::rename(&tmp, dst) {
        Ok(()) => Ok(if existed {
            Replaced::Replaced
        } else {
            Replaced::New
        }),
        Err(first) if existed => {
            let old = with_suffix(dst, OLD_SUFFIX);
            let aside = std::fs::rename(dst, &old).and_then(|()| std::fs::rename(&tmp, dst));
            match aside {
                Ok(()) => Ok(Replaced::MovedAside),
                Err(_) => {
                    let _ = std::fs::remove_file(&tmp);
                    Err(ctx("replace", dst, first))
                }
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(ctx("replace", dst, e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn install_dir_roots() {
        assert_eq!(
            dir_from_local_appdata(Some(r"C:\Users\u\AppData\Local")),
            Some(
                p(r"C:\Users\u\AppData\Local")
                    .join("Programs")
                    .join("OpenSynapse")
            )
        );
        assert_eq!(dir_from_local_appdata(Some(" ")), None);
        assert_eq!(dir_from_local_appdata(None), None);
        assert_eq!(
            dir_from_xdg(Some("/x/data"), Some("/home/u")),
            Some(p("/x/data").join("OpenSynapse").join("bin"))
        );
        assert_eq!(
            dir_from_xdg(Some(""), Some("/home/u")),
            Some(
                p("/home/u")
                    .join(".local")
                    .join("share")
                    .join("OpenSynapse")
                    .join("bin")
            )
        );
        assert_eq!(dir_from_xdg(None, None), None);
        assert_eq!(
            dir_from_home(Some("/home/u")),
            Some(p("/home/u").join(".local").join("bin"))
        );
        assert_eq!(dir_from_home(Some("")), None);
        assert_eq!(dir_from_home(None), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_autostart_plans_use_exec_quoting() {
        let inst = Path::new("/home/u/.local/bin/opensynapse-agent");
        let src = Path::new("/home/u/src/My Repo/target/release");
        let dev = autostart::command_line(&src.join("opensynapse-agent"));
        assert!(dev.starts_with('"'), "{dev}");
        assert!(points_into(&dev, src));
        assert_eq!(
            plan_autostart(Some(&dev), inst, src, true),
            AutostartPlan::Set {
                previous: Some(dev.clone())
            }
        );
        let mine = autostart::command_line(inst);
        assert_eq!(
            plan_autostart(Some(&mine), inst, src, false),
            AutostartPlan::Unchanged
        );
    }

    #[test]
    fn copy_plan_full_and_without_gui() {
        let (src, dst) = (p(r"C:\repo\target\release"), p(r"C:\inst"));
        let all = plan_copy(&src, &dst, |_| true).unwrap();
        assert_eq!(all.copies.len(), 3);
        assert_eq!(all.copies[1], (src.join(AGENT_EXE), dst.join(AGENT_EXE)));
        assert!(all.skipped.is_empty());

        let no_gui = plan_copy(&src, &dst, |f| !f.ends_with(GUI_EXE)).unwrap();
        assert_eq!(no_gui.copies.len(), 2);
        assert_eq!(no_gui.skipped, vec![GUI_EXE]);
    }

    #[test]
    fn copy_plan_errors() {
        let (src, dst) = (p(r"C:\repo\target\release"), p(r"C:\inst"));
        let e = plan_copy(&src, &dst, |f| !f.ends_with(AGENT_EXE)).unwrap_err();
        assert!(e.contains(AGENT_EXE), "{e}");
        assert!(e.contains("--workspace"), "{e}");
        let e = plan_copy(&dst, &dst, |_| true).unwrap_err();
        assert!(e.contains("already runs from"), "{e}");
    }

    #[test]
    fn stale_binaries_get_a_note() {
        let cli = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let min = |m: u64| Duration::from_secs(m * 60);
        // 2026-11-11: GUI 18:49, CLI 20:30 (only the CLI was rebuilt).
        let n = stale_note(GUI_EXE, cli - min(101), cli).unwrap();
        assert!(n.contains(GUI_EXE) && n.contains("101 min"), "{n}");
        assert!(n.contains(BUILD_ALL), "{n}");
        // Same build (linked seconds apart) or newer: no note.
        assert_eq!(stale_note(GUI_EXE, cli - min(1), cli), None);
        assert_eq!(stale_note(GUI_EXE, cli + min(5), cli), None);
    }

    #[cfg(windows)]
    #[test]
    fn paths_compare_case_insensitively_on_windows() {
        assert!(same_path(Path::new(r"C:\Inst\"), Path::new(r"c:\inst")));
        assert!(is_in_dir(
            Path::new(r"C:\INST\x.exe"),
            Path::new(r"c:\inst")
        ));
        assert!(!is_in_dir(
            Path::new(r"C:\inst\sub\x.exe"),
            Path::new(r"c:\inst")
        ));
        assert!(!is_in_dir(
            Path::new(r"C:\inst2\x.exe"),
            Path::new(r"c:\inst")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn autostart_plans() {
        let inst = Path::new(r"C:\inst\opensynapse-agent.exe");
        let src = Path::new(r"C:\repo\target\release");
        let dev = r#""C:\repo\target\release\opensynapse-agent.exe""#;
        let other = r#""D:\other\opensynapse-agent.exe""#;
        let mine = r#""c:\INST\opensynapse-agent.exe""#;
        for flag in [false, true] {
            assert_eq!(
                plan_autostart(Some(mine), inst, src, flag),
                AutostartPlan::Unchanged
            );
            // An entry for the build being installed always moves over.
            assert_eq!(
                plan_autostart(Some(dev), inst, src, flag),
                AutostartPlan::Set {
                    previous: Some(dev.into())
                }
            );
        }
        assert_eq!(
            plan_autostart(None, inst, src, false),
            AutostartPlan::Set { previous: None }
        );
        assert_eq!(
            plan_autostart(None, inst, src, true),
            AutostartPlan::Leave { current: None }
        );
        assert_eq!(
            plan_autostart(Some(other), inst, src, false),
            AutostartPlan::Set {
                previous: Some(other.into())
            }
        );
        assert_eq!(
            plan_autostart(Some(other), inst, src, true),
            AutostartPlan::Leave {
                current: Some(other.into())
            }
        );
    }

    #[test]
    fn start_decision() {
        assert!(start_after_install(true, false));
        assert!(start_after_install(false, true));
        assert!(!start_after_install(false, false));
    }

    #[test]
    fn uninstall_touches_only_known_files_in_dir() {
        let dir = p(r"C:\inst");
        let files = uninstall_files(&dir);
        assert_eq!(files.len(), 9);
        assert!(files.iter().all(|f| is_in_dir(f, &dir)));
        assert!(files.contains(&dir.join(format!("{AGENT_EXE}.old"))));
        assert!(uninstall_refusal(&dir.join(CLI_EXE), &dir).is_some());
        assert!(uninstall_refusal(&p(r"C:\repo").join(CLI_EXE), &dir).is_none());
        let left = leftovers(&dir);
        assert_eq!(left.len(), 6);
        assert!(!left.contains(&dir.join(AGENT_EXE)));
    }

    #[test]
    fn copy_file_replaces_atomically() {
        let dir =
            std::env::temp_dir().join(format!("opensynapse-install-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dst) = (dir.join("src.bin"), dir.join("dst.bin"));
        std::fs::write(&src, b"one").unwrap();
        assert_eq!(copy_file(&src, &dst).unwrap(), Replaced::New);
        std::fs::write(&src, b"two").unwrap();
        assert_eq!(copy_file(&src, &dst).unwrap(), Replaced::Replaced);
        assert_eq!(std::fs::read(&dst).unwrap(), b"two");
        assert!(!with_suffix(&dst, TMP_SUFFIX).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
