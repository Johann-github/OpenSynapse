//! OS integration for OpenSynapse: start the agent at login, refuse to run
//! elevated, reach the parent console from a GUI-subsystem binary, and the
//! per-user install location. Decisions: `vault/Decisions/ADR-0006 Agent
//! autostart.md`, `vault/Decisions/ADR-0009 Install location.md`.
//!
//! Windows only for now; elsewhere [`autostart`] reports `Unsupported`,
//! [`is_elevated`] answers `false` and [`attach_parent_console`] does nothing.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;

/// File name of the agent binary on this OS.
pub const AGENT_EXE: &str = if cfg!(windows) {
    "opensynapse-agent.exe"
} else {
    "opensynapse-agent"
};
/// File name of the CLI binary on this OS.
pub const CLI_EXE: &str = if cfg!(windows) {
    "opensynapse.exe"
} else {
    "opensynapse"
};
/// File name of the desktop GUI binary on this OS.
pub const GUI_EXE: &str = if cfg!(windows) {
    "opensynapse-desktop.exe"
} else {
    "opensynapse-desktop"
};

pub mod install;
pub mod shortcut;
pub mod udev;

/// The agent binary next to the running executable (dev builds: CLI and GUI
/// in `target/release` beside it).
pub fn sibling_agent_exe() -> io::Result<PathBuf> {
    let me = std::env::current_exe()?;
    let dir = me
        .parent()
        .ok_or_else(|| io::Error::other("the executable has no parent directory"))?;
    Ok(dir.join(AGENT_EXE))
}

/// The installed agent ([`install::dir`]), whether or not it exists.
pub fn installed_agent_exe() -> Option<PathBuf> {
    install::dir().map(|d| d.join(AGENT_EXE))
}

/// Pick the agent: the installed one if present, else the sibling (pure).
pub fn resolve_agent_exe(
    installed: Option<PathBuf>,
    sibling: Option<PathBuf>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    [installed, sibling]
        .into_iter()
        .flatten()
        .find(|p| is_file(p))
}

/// The one answer to "which agent binary": the installed agent
/// (`%LOCALAPPDATA%\Programs\OpenSynapse`, ADR-0009) if present, else the one
/// next to the running executable. Used for spawning and for autostart.
pub fn agent_exe() -> io::Result<PathBuf> {
    let sibling = sibling_agent_exe().ok();
    resolve_agent_exe(installed_agent_exe(), sibling.clone(), Path::is_file).ok_or_else(|| {
        let looked: Vec<String> = [installed_agent_exe(), sibling]
            .into_iter()
            .flatten()
            .map(|p| p.display().to_string())
            .collect();
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{AGENT_EXE} not found (looked at {})", looked.join(", ")),
        )
    })
}

/// Exit codes of `opensynapse-agent` that callers react to (owner: the agent's
/// `main.rs`; mirrored here so CLI and GUI read them from one place).
pub mod agent_exit {
    /// Another agent already owns the pipe.
    pub const ALREADY_RUNNING: i32 = 2;
    /// The agent refuses to run with an elevated (administrator) token.
    pub const ELEVATED: i32 = 3;
}

/// Start [`agent_exe`] detached; see [`spawn_detached`].
pub fn spawn_agent_detached() -> io::Result<std::process::Child> {
    spawn_detached(&agent_exe()?, &[])
}

/// Start `exe` with `args` detached: no window, no inherited console, null stdio,
/// its own process group (Ctrl+C in the caller's terminal does not reach it).
/// Does not wait; the caller decides how long to wait for the pipe.
pub fn spawn_detached(exe: &Path, args: &[std::ffi::OsString]) -> io::Result<std::process::Child> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // The detached child must not keep the caller's stdio pipes open.
        windows::stop_std_handle_inheritance();
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("cannot start {}: {e}", exe.display())))
}

/// Another process by pid, to read its image path and wait for its exit without
/// polling (a process handle is signalled when the process ends). Open it
/// *before* asking the process to quit, so the pid cannot be reused meanwhile.
pub struct Process {
    #[cfg(windows)]
    inner: windows::Process,
}

impl Process {
    pub fn open(pid: u32) -> io::Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self {
                inner: windows::Process::open(pid)?,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "waiting for another process is only implemented on Windows so far",
            ))
        }
    }

    /// Full path of the process's executable.
    pub fn image_path(&self) -> io::Result<PathBuf> {
        #[cfg(windows)]
        {
            self.inner.image_path()
        }
        #[cfg(not(windows))]
        {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    /// Block until the process exits or `timeout` passes; `true` if it exited.
    pub fn wait_exit(&self, timeout: std::time::Duration) -> io::Result<bool> {
        #[cfg(windows)]
        {
            self.inner.wait_exit(timeout)
        }
        #[cfg(not(windows))]
        {
            let _ = timeout;
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }
}

/// `true` when this process runs with an elevated (administrator) token.
pub fn is_elevated() -> io::Result<bool> {
    #[cfg(windows)]
    {
        windows::is_elevated()
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

/// For a GUI-subsystem binary started from a terminal: send stdout/stderr to the
/// parent's console. Handles the parent already set (e.g. `2> agent.log`) stay
/// untouched. Returns `true` if a console was attached.
pub fn attach_parent_console() -> bool {
    #[cfg(windows)]
    {
        windows::attach_parent_console()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Per-user configuration files: `%APPDATA%\OpenSynapse\<file>` on Windows,
/// `$XDG_CONFIG_HOME/OpenSynapse` (or `~/.config/OpenSynapse`) elsewhere. One
/// place for the folder, so the agent (`app-profiles.toml`), the CLI and the GUI
/// (`settings.toml`) agree. Read only on demand, never watched.
pub mod config {
    use super::*;

    /// Folder name under the per-user config root.
    pub const DIR_NAME: &str = "OpenSynapse";
    /// User settings of the CLI and GUI (ADR-0008).
    pub const SETTINGS_FILE: &str = "settings.toml";
    /// Per-app profile rules of the agent (ADR-0007).
    pub const APP_PROFILES_FILE: &str = "app-profiles.toml";

    /// `<root>\OpenSynapse` for a config root such as the `APPDATA` value;
    /// `None` when the root is missing or blank.
    pub fn dir_from(root: Option<&str>) -> Option<PathBuf> {
        let root = root.filter(|d| !d.trim().is_empty())?;
        Some(PathBuf::from(root).join(DIR_NAME))
    }

    /// `<root>\OpenSynapsepp-profiles.toml` for a config root such as `APPDATA`.
    pub fn app_profiles_path_from(root: Option<&str>) -> Option<PathBuf> {
        dir_from(root).map(|d| d.join(APP_PROFILES_FILE))
    }

    /// The config folder of the current user, if the environment names one.
    pub fn dir() -> Option<PathBuf> {
        let var = |k: &str| std::env::var(k).ok();
        if cfg!(windows) {
            dir_from(var("APPDATA").as_deref())
        } else {
            dir_from(var("XDG_CONFIG_HOME").as_deref()).or_else(|| {
                let home = var("HOME").filter(|h| !h.trim().is_empty())?;
                Some(PathBuf::from(home).join(".config").join(DIR_NAME))
            })
        }
    }

    /// `settings.toml` in [`dir`].
    pub fn settings_path() -> Option<PathBuf> {
        dir().map(|d| d.join(SETTINGS_FILE))
    }

    /// The file's text, or `None` when it does not exist.
    pub fn read_optional(path: &Path) -> io::Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io::Error::new(
                e.kind(),
                format!("cannot read {}: {e}", path.display()),
            )),
        }
    }

    /// Write `text` to `path`, creating the folder. Writes a sibling temp file and
    /// renames it over the target, so a crash never leaves half a file.
    pub fn write(path: &Path, text: &str) -> io::Result<()> {
        let ctx = |e: io::Error| io::Error::new(e.kind(), format!("{}: {e}", path.display()));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(ctx)?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, text).map_err(ctx)?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            ctx(e)
        })
    }
}

/// Login autostart through the per-user Run key
/// (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value [`VALUE_NAME`]).
/// No admin rights, no service, no scheduled task.
pub mod autostart {
    use super::*;

    /// Registry value name under the Run key.
    pub const VALUE_NAME: &str = "OpenSynapse Agent";

    /// The command stored in the Run value: the quoted executable path.
    pub fn command_line(exe: &Path) -> String {
        format!("\"{}\"", exe.display())
    }

    /// The executable part of a Run command: the quoted part, or everything up
    /// to the first space when unquoted.
    pub fn exe_of(command: &str) -> PathBuf {
        let c = command.trim();
        let exe = match c.strip_prefix('"') {
            Some(rest) => rest.split('"').next().unwrap_or(rest),
            None => c.split(' ').next().unwrap_or(c),
        };
        PathBuf::from(exe)
    }

    /// Does `command` start `exe`? Compared case-insensitively (Windows paths).
    pub fn points_to(command: &str, exe: &Path) -> bool {
        exe_of(command)
            .to_string_lossy()
            .eq_ignore_ascii_case(&exe.to_string_lossy())
    }

    /// The stored command, or `None` when autostart is off.
    pub fn get() -> io::Result<Option<String>> {
        #[cfg(windows)]
        {
            windows::run_get(windows::RUN_KEY)
        }
        #[cfg(not(windows))]
        {
            Err(unsupported())
        }
    }

    /// Start `exe` at login (replaces an existing entry).
    pub fn enable(exe: &Path) -> io::Result<()> {
        #[cfg(windows)]
        {
            windows::run_set(windows::RUN_KEY, &command_line(exe))
        }
        #[cfg(not(windows))]
        {
            let _ = exe;
            Err(unsupported())
        }
    }

    /// Remove the entry. `Ok(false)` if there was none.
    pub fn disable() -> io::Result<bool> {
        #[cfg(windows)]
        {
            windows::run_delete(windows::RUN_KEY)
        }
        #[cfg(not(windows))]
        {
            Err(unsupported())
        }
    }

    #[cfg(not(windows))]
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "autostart is only implemented on Windows so far",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::autostart::*;
    use super::config;
    use std::path::{Path, PathBuf};

    #[test]
    fn agent_exe_prefers_installed() {
        let inst = PathBuf::from(r"C:\inst\opensynapse-agent.exe");
        let sib = PathBuf::from(r"C:\repo\target\release\opensynapse-agent.exe");
        let both = |_: &Path| true;
        // Equality, not `starts_with`: off Windows `C:\repo\…` is one component.
        let only_sib = |p: &Path| p == sib.as_path();
        assert_eq!(
            super::resolve_agent_exe(Some(inst.clone()), Some(sib.clone()), both),
            Some(inst.clone())
        );
        assert_eq!(
            super::resolve_agent_exe(Some(inst.clone()), Some(sib.clone()), only_sib),
            Some(sib.clone())
        );
        assert_eq!(
            super::resolve_agent_exe(None, Some(sib.clone()), both),
            Some(sib)
        );
        assert_eq!(super::resolve_agent_exe(Some(inst), None, |_| false), None);
    }

    #[test]
    fn config_dir_from_root() {
        assert_eq!(
            config::dir_from(Some(r"C:\Users\u\AppData\Roaming")),
            Some(PathBuf::from(r"C:\Users\u\AppData\Roaming").join("OpenSynapse"))
        );
        assert_eq!(config::dir_from(Some("  ")), None);
        assert_eq!(config::dir_from(None), None);
    }

    #[test]
    fn config_read_write_round_trip() {
        let dir = std::env::temp_dir().join(format!("opensynapse-sys-test-{}", std::process::id()));
        let file = dir.join("sub").join("settings.toml");
        assert_eq!(config::read_optional(&file).unwrap(), None);
        config::write(&file, "wake_hint = true\n").unwrap();
        config::write(&file, "wake_hint = false\n").unwrap();
        assert_eq!(
            config::read_optional(&file).unwrap().as_deref(),
            Some("wake_hint = false\n")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn command_line_quotes_the_path() {
        let exe = Path::new(r"C:\Program Files\OpenSynapse\opensynapse-agent.exe");
        let c = command_line(exe);
        assert_eq!(c, r#""C:\Program Files\OpenSynapse\opensynapse-agent.exe""#);
        assert_eq!(exe_of(&c), exe);
        assert!(points_to(&c, exe));
    }

    #[test]
    fn exe_of_variants() {
        assert_eq!(
            exe_of(r#""C:\a b\x.exe" --flag"#),
            Path::new(r"C:\a b\x.exe")
        );
        assert_eq!(exe_of(r"C:\x.exe --flag"), Path::new(r"C:\x.exe"));
        assert_eq!(exe_of(r"  C:\x.exe  "), Path::new(r"C:\x.exe"));
        assert_eq!(exe_of(r#""C:\unterminated"#), Path::new(r"C:\unterminated"));
    }

    #[test]
    fn points_to_ignores_case_only() {
        let exe = Path::new(r"C:\Tools\opensynapse-agent.exe");
        assert!(points_to(r#""c:\tools\OPENSYNAPSE-AGENT.EXE""#, exe));
        assert!(!points_to(r#""C:\Other\opensynapse-agent.exe""#, exe));
    }
}
