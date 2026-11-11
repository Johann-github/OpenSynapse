//! `opensynapse agent start` and `opensynapse agent autostart [on|off]`
//! (`vault/Decisions/ADR-0006 Agent autostart.md`). The agent binary is the
//! installed one if present, else the one next to this executable
//! (`opensynapse_sys::agent_exe`, ADR-0009); autostart is the per-user Run key.

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use opensynapse_ipc::Client;
use opensynapse_sys::{agent_exit, autostart};

/// How long `start` waits for the new agent to answer. Startup only, not idle.
const START_WAIT: Duration = Duration::from_secs(3);
const START_STEP: Duration = Duration::from_millis(50);

/// `on` / `off` for `agent autostart`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Toggle {
    On,
    Off,
}

/// Start the agent detached (no window, no inherited console) and wait until its
/// pipe answers. The binary is `opensynapse_sys::agent_exe` (installed first).
pub fn start() -> Result<()> {
    let pipe = crate::agent::pipe_name()?;
    if Client::connect_to(&pipe).is_ok() {
        println!("OpenSynapse agent is already running.");
        return Ok(());
    }
    let exe = opensynapse_sys::agent_exe().context("cannot start the agent")?;
    println!("{}", start_and_wait(&exe, &pipe)?);
    Ok(())
}

/// Arguments for a spawned agent: `--pipe <name>` when `OPENSYNAPSE_AGENT_PIPE`
/// points the CLI at another pipe, so the new agent serves the same one.
pub(crate) fn agent_args(pipe_override: Option<&str>) -> Vec<OsString> {
    match pipe_override {
        Some(p) => vec!["--pipe".into(), p.into()],
        None => Vec::new(),
    }
}

/// Spawn `exe` detached and wait until `pipe` answers. Returns the status line.
pub(crate) fn start_and_wait(exe: &Path, pipe: &str) -> Result<String> {
    let args = agent_args(crate::agent::pipe_override().as_deref());
    let mut child =
        opensynapse_sys::spawn_detached(exe, &args).context("cannot start the agent")?;
    let deadline = Instant::now() + START_WAIT;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.code() == Some(agent_exit::ALREADY_RUNNING) {
                return Ok("OpenSynapse agent is already running.".into());
            }
            bail!(render_exit(status));
        }
        if Client::connect_to(pipe).is_ok() {
            return Ok(format!(
                "OpenSynapse agent started (pid {}): {}",
                child.id(),
                exe.display()
            ));
        }
        if Instant::now() >= deadline {
            bail!("the agent (pid {}) did not answer within 3 s", child.id());
        }
        std::thread::sleep(START_STEP);
    }
}

/// Why a freshly started agent exited.
pub(crate) fn render_exit(status: ExitStatus) -> String {
    match status.code() {
        Some(agent_exit::ELEVATED) => "the agent refuses to run as administrator; \
             run this command from a normal (non-elevated) terminal"
            .into(),
        _ => format!("the agent exited right away ({status}); run opensynapse-agent -v to see why"),
    }
}

/// Show, enable or disable the login autostart.
pub fn autostart(state: Option<Toggle>) -> Result<()> {
    match state {
        None => {
            let current = autostart::get().context("cannot read the autostart entry")?;
            let exe = opensynapse_sys::agent_exe().ok();
            println!("{}", render_status(current.as_deref(), exe.as_deref()));
        }
        Some(Toggle::On) => {
            let exe = opensynapse_sys::agent_exe().context("cannot find the agent binary")?;
            autostart::enable(&exe).context("cannot write the autostart entry")?;
            println!(
                "Autostart on: {} starts at login. Start it now with `opensynapse agent start`.",
                exe.display()
            );
        }
        Some(Toggle::Off) => {
            if autostart::disable().context("cannot remove the autostart entry")? {
                println!(
                    "Autostart off. A running agent keeps running (`opensynapse agent stop`)."
                );
            } else {
                println!("Autostart was already off.");
            }
        }
    }
    Ok(())
}

/// Status line for `agent autostart`. `exe` is the agent next to this CLI.
pub(crate) fn render_status(current: Option<&str>, exe: Option<&Path>) -> String {
    match (current, exe) {
        (None, _) => "Autostart off.".into(),
        (Some(cmd), Some(exe)) if autostart::points_to(cmd, exe) => format!("Autostart on: {cmd}"),
        (Some(cmd), _) => format!(
            "Autostart on, but for another binary: {cmd}\n\
             Run `opensynapse agent autostart on` to point it at this installation."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        let exe = Path::new(r"C:\os\opensynapse-agent.exe");
        assert_eq!(render_status(None, Some(exe)), "Autostart off.");
        assert_eq!(
            render_status(Some(r#""C:\os\opensynapse-agent.exe""#), Some(exe)),
            r#"Autostart on: "C:\os\opensynapse-agent.exe""#
        );
        assert!(
            render_status(Some(r#""D:\old\opensynapse-agent.exe""#), Some(exe))
                .starts_with("Autostart on, but for another binary")
        );
        assert!(
            render_status(Some(r#""C:\os\opensynapse-agent.exe""#), None)
                .starts_with("Autostart on, but for another binary")
        );
    }

    #[test]
    fn spawned_agent_follows_the_pipe_override() {
        assert!(agent_args(None).is_empty());
        assert_eq!(
            agent_args(Some("p")),
            vec![OsString::from("--pipe"), "p".into()]
        );
    }

    #[cfg(windows)]
    #[test]
    fn exit_messages() {
        use std::os::windows::process::ExitStatusExt;
        assert!(render_exit(ExitStatus::from_raw(3)).contains("administrator"));
        assert!(render_exit(ExitStatus::from_raw(1)).contains("opensynapse-agent -v"));
    }
}
