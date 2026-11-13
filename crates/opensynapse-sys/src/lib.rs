//! OS integration for OpenSynapse: start the agent at login, refuse to run
//! elevated, reach the parent console from a GUI-subsystem binary, and the
//! per-user install location. Decisions: `vault/Decisions/ADR-0006 Agent
//! autostart.md`, `vault/Decisions/ADR-0009 Install location.md`,
//! `vault/Decisions/ADR-0011 Linux autostart and install.md`,
//! `vault/Decisions/ADR-0012 macOS agent.md`.
//!
//! Windows, Linux and macOS. On Linux, [`autostart`] is an XDG autostart entry;
//! on macOS a launchd LaunchAgent plist; on both [`is_elevated`] means
//! "effective user is root". On other systems [`autostart`] reports
//! `Unsupported` and [`is_elevated`] answers `false`; [`attach_parent_console`]
//! does nothing outside Windows.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
mod macos;
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

pub mod desktop_entry;
pub mod install;
pub mod launch_agent;
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
    /// The agent refuses to run with an elevated (administrator) token, or as
    /// root on Linux.
    pub const ELEVATED: i32 = 3;
}

/// What "elevated" means on this OS, for messages: `administrator` on Windows,
/// `root` elsewhere.
pub const ELEVATED_NAME: &str = if cfg!(windows) {
    "administrator"
} else {
    "root"
};

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

/// `true` when this process runs with an elevated (administrator) token; on
/// Linux and macOS, when its effective user id is 0 (root). Linux reads the
/// `Uid:` line of `/proc/self/status` with std; macOS has no `/proc` and calls
/// libSystem's `geteuid` through a one-line declaration. No `libc` crate either way.
pub fn is_elevated() -> io::Result<bool> {
    #[cfg(windows)]
    {
        windows::is_elevated()
    }
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status")?;
        desktop_entry::euid_from_proc_status(&status)
            .map(|euid| euid == 0)
            .ok_or_else(|| io::Error::other("no Uid line in /proc/self/status"))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(macos::is_root())
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
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

/// Login autostart, per user, no admin rights. Windows: the Run key value
/// [`VALUE_NAME`] under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
/// (no service, no scheduled task). Linux: the XDG autostart entry
/// `$XDG_CONFIG_HOME/autostart/opensynapse-agent.desktop`, else under
/// `~/.config` (no systemd unit; ADR-0011). macOS: the LaunchAgent
/// `~/Library/LaunchAgents/<label>.plist`, read by launchd at the next login
/// (no `launchctl`; ADR-0012).
///
/// The "command" handed around is the Run value on Windows, the entry's
/// `Exec=` value (Desktop Entry quoting, string escapes undone) on Linux, and
/// the plain agent path (first `ProgramArguments` string) on macOS.
pub mod autostart {
    use super::*;

    /// How this OS stores the autostart command.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Form {
        /// Quoted Run value (Windows and fallback); paths compare ignoring case.
        Windows,
        /// Desktop Entry `Exec=` value.
        Linux,
        /// Plain path from the plist.
        MacOs,
    }

    const FORM: Form = if cfg!(target_os = "linux") {
        Form::Linux
    } else if cfg!(target_os = "macos") {
        Form::MacOs
    } else {
        Form::Windows
    };

    /// Registry value name under the Run key.
    pub const VALUE_NAME: &str = "OpenSynapse Agent";

    /// The command stored in the Run value: the quoted executable path.
    pub fn windows_command_line(exe: &Path) -> String {
        format!("\"{}\"", exe.display())
    }

    /// The executable part of a Run command: the quoted part, or everything up
    /// to the first space when unquoted.
    pub fn windows_exe_of(command: &str) -> PathBuf {
        let c = command.trim();
        let exe = match c.strip_prefix('"') {
            Some(rest) => rest.split('"').next().unwrap_or(rest),
            None => c.split(' ').next().unwrap_or(c),
        };
        PathBuf::from(exe)
    }

    /// The command that starts `exe` at login on this OS: the Run value on
    /// Windows, the `Exec=` value on Linux, the path itself on macOS.
    pub fn command_line(exe: &Path) -> String {
        match FORM {
            Form::Linux => desktop_entry::quote_exec_arg(&exe.to_string_lossy()),
            Form::MacOs => exe.to_string_lossy().into_owned(),
            Form::Windows => windows_command_line(exe),
        }
    }

    /// The executable a stored command starts.
    pub fn exe_of(command: &str) -> PathBuf {
        match FORM {
            Form::Linux => PathBuf::from(desktop_entry::exec_program(command)),
            Form::MacOs => PathBuf::from(command),
            Form::Windows => windows_exe_of(command),
        }
    }

    /// Does `command` start `exe`? Case-insensitive on Windows only (on macOS
    /// a case-only mismatch merely rewrites our plist).
    pub fn points_to(command: &str, exe: &Path) -> bool {
        let found = exe_of(command).to_string_lossy().into_owned();
        let want = exe.to_string_lossy();
        match FORM {
            Form::Linux | Form::MacOs => found == want,
            Form::Windows => found.eq_ignore_ascii_case(&want),
        }
    }

    /// The stored command, or `None` when autostart is off (on Linux also when
    /// our entry is switched off with `Hidden=true` or
    /// `X-GNOME-Autostart-enabled=false`; on macOS when our plist has
    /// `Disabled` true or no `RunAtLoad` true).
    pub fn get() -> io::Result<Option<String>> {
        #[cfg(windows)]
        {
            windows::run_get(windows::RUN_KEY)
        }
        #[cfg(target_os = "linux")]
        {
            Ok(config::read_optional(&linux_path()?)?
                .as_deref()
                .and_then(desktop_entry::autostart_exec))
        }
        #[cfg(target_os = "macos")]
        {
            Ok(config::read_optional(&macos_path()?)?
                .as_deref()
                .and_then(launch_agent::program))
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            Err(unsupported())
        }
    }

    /// Start `exe` at login (replaces an existing entry). Linux and macOS:
    /// written as a temp file + rename ([`config::write`]).
    pub fn enable(exe: &Path) -> io::Result<()> {
        #[cfg(windows)]
        {
            windows::run_set(windows::RUN_KEY, &command_line(exe))
        }
        #[cfg(target_os = "linux")]
        {
            config::write(&linux_path()?, &desktop_entry::render_autostart(utf8(exe)?))
        }
        #[cfg(target_os = "macos")]
        {
            let text = launch_agent::render(utf8(exe)?)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            config::write(&macos_path()?, &text)
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
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
        #[cfg(target_os = "linux")]
        {
            remove(&linux_path()?)
        }
        #[cfg(target_os = "macos")]
        {
            remove(&macos_path()?)
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            Err(unsupported())
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn utf8(exe: &Path) -> io::Result<&str> {
        exe.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not valid UTF-8", exe.display()),
            )
        })
    }

    /// Delete our entry file; `Ok(false)` when it was not there.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn remove(path: &Path) -> io::Result<bool> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io::Error::new(
                e.kind(),
                format!("cannot remove {}: {e}", path.display()),
            )),
        }
    }

    /// The current user's autostart entry.
    #[cfg(target_os = "linux")]
    fn linux_path() -> io::Result<PathBuf> {
        let var = |k: &str| std::env::var(k).ok();
        desktop_entry::autostart_path(var("XDG_CONFIG_HOME").as_deref(), var("HOME").as_deref())
            .ok_or_else(|| io::Error::other("cannot locate the autostart folder (HOME is not set)"))
    }

    /// The current user's LaunchAgent plist.
    #[cfg(target_os = "macos")]
    fn macos_path() -> io::Result<PathBuf> {
        launch_agent::plist_path(std::env::var("HOME").ok().as_deref()).ok_or_else(|| {
            io::Error::other("cannot locate ~/Library/LaunchAgents (HOME is not set)")
        })
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "autostart is only implemented on Windows, Linux and macOS so far",
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
    fn windows_command_line_quotes_the_path() {
        let exe = Path::new(r"C:\Program Files\OpenSynapse\opensynapse-agent.exe");
        let c = windows_command_line(exe);
        assert_eq!(c, r#""C:\Program Files\OpenSynapse\opensynapse-agent.exe""#);
        assert_eq!(windows_exe_of(&c), exe);
    }

    #[test]
    fn windows_exe_of_variants() {
        assert_eq!(
            windows_exe_of(r#""C:\a b\x.exe" --flag"#),
            Path::new(r"C:\a b\x.exe")
        );
        assert_eq!(windows_exe_of(r"C:\x.exe --flag"), Path::new(r"C:\x.exe"));
        assert_eq!(windows_exe_of(r"  C:\x.exe  "), Path::new(r"C:\x.exe"));
        assert_eq!(
            windows_exe_of(r#""C:\unterminated"#),
            Path::new(r"C:\unterminated")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_command_is_the_exec_value() {
        let exe = Path::new("/home/u/My Apps/opensynapse-agent");
        let c = command_line(exe);
        assert_eq!(c, "\"/home/u/My Apps/opensynapse-agent\"");
        assert_eq!(exe_of(&c), exe);
        assert!(points_to(&c, exe));
        assert!(!points_to(
            &c,
            Path::new("/home/u/my apps/opensynapse-agent")
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_command_is_the_plain_path() {
        let exe = Path::new("/Users/u/My Apps/opensynapse-agent");
        let c = command_line(exe);
        assert_eq!(c, "/Users/u/My Apps/opensynapse-agent");
        assert_eq!(exe_of(&c), exe);
        assert!(points_to(&c, exe));
        let plist = super::launch_agent::render(&c).unwrap();
        assert_eq!(
            super::launch_agent::program(&plist).as_deref(),
            Some(c.as_str())
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn points_to_ignores_case_only() {
        let c = command_line(Path::new(r"C:\x\a.exe"));
        assert_eq!(exe_of(&c), Path::new(r"C:\x\a.exe"));
        let exe = Path::new(r"C:\Tools\opensynapse-agent.exe");
        assert!(points_to(r#""c:\tools\OPENSYNAPSE-AGENT.EXE""#, exe));
        assert!(!points_to(r#""C:\Other\opensynapse-agent.exe""#, exe));
    }
}
