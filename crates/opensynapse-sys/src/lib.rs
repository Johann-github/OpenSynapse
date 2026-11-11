//! OS integration for OpenSynapse: start the agent at login, refuse to run
//! elevated, and reach the parent console from a GUI-subsystem binary.
//! Decision: `vault/Decisions/ADR-0006 Agent autostart.md`.
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

/// The agent binary next to the running executable (CLI and GUI ship beside it).
pub fn agent_exe() -> io::Result<PathBuf> {
    let me = std::env::current_exe()?;
    let dir = me
        .parent()
        .ok_or_else(|| io::Error::other("the executable has no parent directory"))?;
    let exe = dir.join(AGENT_EXE);
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} not found", exe.display()),
        ))
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
    use std::path::Path;

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
