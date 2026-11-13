//! Agent controls in the top bar: login autostart and "Start agent". Pure state
//! and text; the registry calls and the spawn happen in [`crate::app`] and
//! [`crate::worker`] (`vault/Decisions/ADR-0006 Agent autostart.md`).

use std::io;
use std::path::Path;
use std::time::Duration;

use opensynapse_sys::{agent_exit, autostart};

/// How long the worker waits for a freshly started agent to answer. Startup
/// only, never while idle.
pub const START_WAIT: Duration = Duration::from_secs(3);
pub const START_STEP: Duration = Duration::from_millis(50);

/// What the "Start agent at login" checkbox shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutostartState {
    Off,
    /// On, for the agent this GUI uses (installed, else next to the GUI).
    On,
    /// On, but the Run value starts another binary (stored command).
    Other(String),
    /// Could not read the Run value.
    Error(String),
}

impl AutostartState {
    /// From `autostart::get()` and the agent this GUI uses (`opensynapse_sys::agent_exe`, if found).
    pub fn from_read(current: io::Result<Option<String>>, exe: Option<&Path>) -> Self {
        match current {
            Ok(None) => Self::Off,
            Ok(Some(cmd)) => match exe {
                Some(exe) if autostart::points_to(&cmd, exe) => Self::On,
                _ => Self::Other(cmd),
            },
            Err(e) => Self::Error(format!("cannot read the autostart entry: {e}")),
        }
    }

    /// Checkbox value: an entry exists.
    pub fn is_on(&self) -> bool {
        matches!(self, Self::On | Self::Other(_))
    }
}

/// Why the checkbox is disabled, if it is: without an agent binary (installed or next to the GUI)
/// there is nothing to register; elsewhere than Windows it is not implemented.
pub fn autostart_blocked(state: &AutostartState, exe_found: bool) -> Option<String> {
    if !exe_found {
        return Some(format!(
            "{} not found (neither installed nor next to the GUI); autostart registers that binary",
            opensynapse_sys::AGENT_EXE
        ));
    }
    match state {
        AutostartState::Error(e) => Some(e.clone()),
        _ => None,
    }
}

/// Warning line for an entry that starts another binary.
pub fn other_binary_note(cmd: &str) -> String {
    format!("Autostart starts another binary: {cmd}")
}

/// Status line after toggling.
pub fn toggle_result(on: bool, r: io::Result<bool>) -> Result<String, String> {
    match (on, r) {
        (true, Ok(_)) => Ok("Autostart on: the agent starts at your next login.".into()),
        (false, Ok(true)) => Ok("Autostart off. A running agent keeps running.".into()),
        (false, Ok(false)) => Ok("Autostart was already off.".into()),
        (true, Err(e)) => Err(format!("cannot write the autostart entry: {e}")),
        (false, Err(e)) => Err(format!("cannot remove the autostart entry: {e}")),
    }
}

/// How a "Start agent" attempt ended before the agent answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartFailure {
    /// Spawning failed (binary missing, access denied, ...).
    Spawn(String),
    /// The agent exited with this code (`None`: killed / no code).
    Exited(Option<i32>),
    /// Still running but no answer within [`START_WAIT`].
    NoAnswer { pid: u32 },
}

/// The note shown next to "Backend: direct HID" after a failed start.
pub fn start_failure_note(f: &StartFailure) -> String {
    match f {
        StartFailure::Spawn(e) => format!("cannot start the agent: {e}"),
        StartFailure::Exited(Some(agent_exit::ELEVATED)) => format!(
            "the agent refuses to run as {}; start the GUI unelevated",
            opensynapse_sys::ELEVATED_NAME
        ),
        StartFailure::Exited(Some(code)) => format!(
            "the agent exited right away (code {code}); run opensynapse-agent -v to see why"
        ),
        StartFailure::Exited(None) => {
            "the agent exited right away; run opensynapse-agent -v to see why".into()
        }
        StartFailure::NoAnswer { pid } => format!(
            "the agent (pid {pid}) did not answer within {} s; try \"Reconnect to agent\"",
            START_WAIT.as_secs()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\os\opensynapse-agent.exe";

    #[test]
    fn autostart_state_from_registry() {
        let exe = Path::new(EXE);
        assert_eq!(
            AutostartState::from_read(Ok(None), Some(exe)),
            AutostartState::Off
        );
        let mine = format!("\"{EXE}\"");
        assert_eq!(
            AutostartState::from_read(Ok(Some(mine.clone())), Some(exe)),
            AutostartState::On
        );
        let other = r#""D:\old\opensynapse-agent.exe""#.to_owned();
        assert_eq!(
            AutostartState::from_read(Ok(Some(other.clone())), Some(exe)),
            AutostartState::Other(other)
        );
        // No agent binary found: any entry is "another binary".
        assert_eq!(
            AutostartState::from_read(Ok(Some(mine.clone())), None),
            AutostartState::Other(mine)
        );
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let s = AutostartState::from_read(Err(denied), Some(exe));
        assert!(matches!(&s, AutostartState::Error(e) if e.contains("denied")));
        assert!(!s.is_on());
        assert!(AutostartState::On.is_on());
        assert!(AutostartState::Other("x".into()).is_on());
        assert!(!AutostartState::Off.is_on());
    }

    #[test]
    fn checkbox_blocked_without_agent_or_on_error() {
        assert!(autostart_blocked(&AutostartState::Off, true).is_none());
        assert!(autostart_blocked(&AutostartState::Other("x".into()), true).is_none());
        let missing = autostart_blocked(&AutostartState::Off, false).unwrap();
        assert!(missing.contains(opensynapse_sys::AGENT_EXE), "{missing}");
        let err = AutostartState::Error("cannot read: Unsupported".into());
        assert!(
            autostart_blocked(&err, true)
                .unwrap()
                .contains("Unsupported")
        );
    }

    #[test]
    fn toggle_messages() {
        assert!(
            toggle_result(true, Ok(true))
                .unwrap()
                .starts_with("Autostart on")
        );
        assert!(
            toggle_result(false, Ok(true))
                .unwrap()
                .starts_with("Autostart off")
        );
        assert!(
            toggle_result(false, Ok(false))
                .unwrap()
                .contains("already off")
        );
        let e = || io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        assert!(toggle_result(true, Err(e())).unwrap_err().contains("write"));
        assert!(
            toggle_result(false, Err(e()))
                .unwrap_err()
                .contains("remove")
        );
        assert!(other_binary_note("\"D:\\x.exe\"").contains("another binary"));
    }

    #[test]
    fn start_failures() {
        let elevated = StartFailure::Exited(Some(agent_exit::ELEVATED));
        assert!(start_failure_note(&elevated).contains(&format!(
            "refuses to run as {}",
            opensynapse_sys::ELEVATED_NAME
        )));
        assert!(start_failure_note(&StartFailure::Exited(Some(1))).contains("code 1"));
        assert!(start_failure_note(&StartFailure::Exited(None)).contains("-v"));
        assert!(start_failure_note(&StartFailure::NoAnswer { pid: 42 }).contains("pid 42"));
        assert!(start_failure_note(&StartFailure::Spawn("not found".into())).contains("not found"));
    }
}
