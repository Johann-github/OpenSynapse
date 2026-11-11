//! OpenSynapse background agent: owns the HID devices and serves clients (CLI,
//! GUI) over a per-user named pipe. Decision: `vault/Decisions/ADR-0005 Agent IPC.md`.
//!
//! Threads: the core actor on the main thread (owns every HID session), one
//! accept thread, one thread per connection. Hotplug comes from the OS; nothing
//! runs on a timer, so the idle agent uses no CPU.
//!
//! On Windows it is a GUI-subsystem binary, so the login autostart opens no
//! console window; started from a terminal it attaches to that console for its
//! logs. It refuses to run elevated (`vault/Decisions/ADR-0006 Agent autostart.md`).

#![cfg_attr(windows, windows_subsystem = "windows")]

mod convert;
mod core;
mod events;
mod hardware;
mod server;

use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::time::Duration;

use opensynapse_device_registry::RAZER_VID;
use opensynapse_ipc::codec::{decode_server, encode_client};
use opensynapse_ipc::frame::{read_frame, write_frame};
use opensynapse_ipc::msg::{ClientFrame, PROTOCOL_VERSION, Role, ServerFrame};
use opensynapse_ipc::transport::{self, Listener};

use crate::core::{Core, CoreMsg, SETTLE};
use crate::events::{EventSource, HidInputEvents, HidOpener, NoEvents, default_candidate};
use crate::hardware::RealHardware;

const USAGE: &str = "usage: opensynapse-agent [--pipe <name>] [--no-events] [-v | -vv]\n  \
     --no-events  do not listen for device hardware events (also OPENSYNAPSE_AGENT_NO_EVENTS=1)";
/// Environment switch equivalent to `--no-events`.
const NO_EVENTS_ENV: &str = "OPENSYNAPSE_AGENT_NO_EVENTS";
/// Exit code when another agent already serves the pipe.
const EXIT_RUNNING: u8 = 2;
/// Exit code when started elevated without `--allow-elevated`.
const EXIT_ELEVATED: u8 = 3;
/// How long shutdown waits for the `ShuttingDown` reply to be written.
const FLUSH_WAIT: Duration = Duration::from_secs(1);

#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    pipe: Option<String>,
    verbose: u8,
    /// Use `NoEvents` instead of `HidInputEvents`. Events are on by default:
    /// the Viper V3 HyperSpeed sends report 5 on MI_01 COL06 (observed,
    /// `vault/RE-Log/2026-11-10 Hardware events capture.md`).
    no_events: bool,
    /// Skip the elevation refusal.
    allow_elevated: bool,
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args::default();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pipe" => a.pipe = Some(it.next().ok_or("--pipe needs a name")?),
            "--no-events" => a.no_events = true,
            "--allow-elevated" => a.allow_elevated = true,
            "-v" | "--verbose" => a.verbose += 1,
            "-vv" => a.verbose += 2,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }
    Ok(a)
}

/// `1`, `true`, `yes`, `on` (any case) switch an environment flag on.
fn env_flag(v: Option<&str>) -> bool {
    v.is_some_and(|v| {
        ["1", "true", "yes", "on"]
            .iter()
            .any(|t| v.trim().eq_ignore_ascii_case(t))
    })
}

fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}

/// Handshake with the agent that already serves `name`; returns its PID.
fn running_agent_pid(name: &str) -> io::Result<u32> {
    let mut conn = transport::connect(name)?;
    let hello = ClientFrame::Hello {
        version: PROTOCOL_VERSION,
        role: Role::Control,
    };
    write_frame(&mut conn, &encode_client(&hello))?;
    match decode_server(&read_frame(&mut conn)?) {
        Ok(ServerFrame::Welcome { agent_pid, .. }) => Ok(agent_pid),
        Ok(other) => Err(io::Error::other(format!("unexpected answer {other:?}"))),
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
    }
}

fn main() -> ExitCode {
    // Before anything prints: a GUI-subsystem process has no stderr otherwise.
    opensynapse_sys::attach_parent_console();
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    init_tracing(args.verbose);

    // An elevated agent would serve unelevated clients with admin rights, and the
    // Run-key autostart never starts it elevated anyway.
    if !args.allow_elevated && opensynapse_sys::is_elevated().unwrap_or(false) {
        eprintln!(
            "opensynapse-agent: refusing to run as administrator; start it from a normal              (non-elevated) session, or pass --allow-elevated on systems without UAC"
        );
        return ExitCode::from(EXIT_ELEVATED);
    }

    let name = match args.pipe {
        Some(n) => n,
        None => match transport::default_pipe_name() {
            Ok(n) => n,
            Err(e) => {
                tracing::error!(error = %e, "cannot determine the pipe name");
                return ExitCode::FAILURE;
            }
        },
    };

    // Bind first: the pipe is the single-instance lock.
    let listener = match Listener::bind(&name) {
        Ok(l) => l,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            match running_agent_pid(&name) {
                Ok(pid) => eprintln!("opensynapse-agent: already running (pid {pid})"),
                Err(_) => eprintln!("opensynapse-agent: already running"),
            }
            return ExitCode::from(EXIT_RUNNING);
        }
        Err(e) => {
            tracing::error!(error = %e, pipe = %name, "cannot create the pipe");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(pipe = %name, pid = std::process::id(), "agent listening");

    let (tx, rx) = channel::<CoreMsg>();
    let pending = Arc::new(AtomicBool::new(false));

    // Merge a burst of notifications into one rescan.
    let hotplug = {
        let tx = tx.clone();
        let pending = pending.clone();
        opensynapse_hid::hotplug::watch(RAZER_VID, move |ev| {
            tracing::debug!(?ev, "hotplug");
            if !pending.swap(true, Ordering::SeqCst) {
                let _ = tx.send(CoreMsg::Rescan);
            }
        })
    };
    let hotplug = match hotplug {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!(error = %e, "no hotplug notifications; rescanning on ListDevices");
            None
        }
    };

    let events_off = args.no_events || env_flag(std::env::var(NO_EVENTS_ENV).ok().as_deref());
    let events: Box<dyn EventSource> = if !events_off {
        tracing::info!("hardware events on");
        Box::new(HidInputEvents::new(HidOpener, default_candidate))
    } else {
        Box::new(NoEvents)
    };
    let mut core = Core::new(RealHardware::default(), events, tx.clone(), pending, SETTLE);
    core.set_rescan_on_list(hotplug.is_none());
    core.rescan();

    let stop = Arc::new(AtomicBool::new(false));
    let waker = listener.waker();
    let (flushed_tx, flushed_rx) = channel();
    let accept = match server::spawn(listener, tx, stop.clone(), flushed_tx) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "cannot start the accept thread");
            return ExitCode::FAILURE;
        }
    };

    core.run(&rx);

    tracing::info!("shutting down");
    // Let the connection that asked for shutdown get its reply out.
    let _ = flushed_rx.recv_timeout(FLUSH_WAIT);
    stop.store(true, Ordering::SeqCst);
    waker.wake();
    let _ = accept.join();
    drop(hotplug);
    drop(core);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Result<Args, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn args() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
        assert_eq!(
            parse(&["--pipe", "x", "-v"]).unwrap(),
            Args {
                pipe: Some("x".into()),
                verbose: 1,
                no_events: false,
                allow_elevated: false,
            }
        );
        assert_eq!(parse(&["-vv"]).unwrap().verbose, 2);
        assert!(parse(&["--pipe"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
        assert!(parse(&["--help"]).is_err());
        assert!(parse(&["--no-events"]).unwrap().no_events);
        assert!(parse(&["--events"]).is_err());
        assert!(parse(&["--allow-elevated"]).unwrap().allow_elevated);
    }

    #[test]
    fn env_flag_values() {
        assert!(env_flag(Some("1")));
        assert!(env_flag(Some(" TRUE ")));
        assert!(!env_flag(Some("0")));
        assert!(!env_flag(Some("")));
        assert!(!env_flag(None));
    }
}
