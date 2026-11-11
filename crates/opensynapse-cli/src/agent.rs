//! `opensynapse agent …`: talk to a running `opensynapse-agent` over local IPC
//! (`vault/Decisions/ADR-0005 Agent IPC.md`): read-only commands, `stop`, and
//! `apply` (profile apply through the agent's shared engine; the restore file is
//! written here, client-side).
//!
//! All output is built by the pure `render_*` functions so it can be tested without
//! a running agent; [`run`] only connects, calls and prints.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use opensynapse_ipc::Client;
use opensynapse_ipc::msg::{
    AppProfilesInfo, DeviceInfo, DeviceRef, ErrorKind, Event, Field, IpcError, PollingVariant,
    Reply, Request, WireSetting, WireSnapshot, WireWriteOpts,
};
use opensynapse_ipc::transport::default_pipe_name;
use opensynapse_protocol::power::{BatteryLevel, ChargingStatus};

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum AgentAction {
    /// Check that the agent runs and show its version.
    Ping,
    /// List the devices the agent sees.
    Devices,
    /// Read all settings of one device through the agent. Select it with the global
    /// --pid or with --path; without either, the only connected device is used.
    Snapshot {
        /// Control collection path, as shown by `agent devices`.
        #[arg(long, conflicts_with = "pid")]
        path: Option<String>,
    },
    /// Apply a profile through the agent (same engine as `profile apply`). Without
    /// --apply this only shows what would be sent. With --apply, the values from
    /// before are saved to `<name>.restore.toml` (by this command, not the agent).
    Apply {
        /// Profile TOML file.
        file: PathBuf,
        /// Actually write to the device.
        #[arg(long)]
        apply: bool,
        /// Also write settings that already have the profile's value (normally skipped).
        #[arg(long)]
        all: bool,
        /// Write even if Razer software is running, the profile is for another PID,
        /// or the snapshot could not undo every change.
        #[arg(long)]
        force: bool,
        /// Control collection path, as shown by `agent devices`.
        #[arg(long, conflicts_with = "pid")]
        path: Option<String>,
    },
    /// Print agent events (hotplug, battery, …) as they arrive, until the agent stops.
    Watch,
    /// Per-app profiles: show the loaded rules and the active profile.
    /// Rules file: %APPDATA%\OpenSynapse\app-profiles.toml.
    AppProfiles {
        #[command(subcommand)]
        action: Option<AppProfilesAction>,
    },
    /// Ask the agent to shut down.
    Stop,
    /// Start the agent in the background (no window) and wait until it answers.
    Start,
    /// Show the login autostart, or turn it on/off (per-user Run key, no admin rights).
    Autostart {
        /// `on` or `off`; without it, show the current state.
        #[arg(value_enum)]
        state: Option<crate::lifecycle::Toggle>,
    },
}

#[derive(Subcommand, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppProfilesAction {
    /// Re-read app-profiles.toml and apply what it says for the app in front.
    Reload,
}

/// Environment override for the pipe name (tests against a second agent).
const PIPE_ENV: &str = "OPENSYNAPSE_AGENT_PIPE";

/// Run one agent command. `pid` is the global `--pid`.
pub fn run(action: AgentAction, pid: Option<u16>) -> Result<()> {
    match action {
        AgentAction::Watch => watch(),
        AgentAction::Start => crate::lifecycle::start(),
        AgentAction::Autostart { state } => crate::lifecycle::autostart(state),
        AgentAction::Ping => {
            let r = call(&mut connect()?, Request::Ping)?;
            println!("{}", render_reply(&r));
            Ok(())
        }
        AgentAction::Devices => {
            let r = call(&mut connect()?, Request::ListDevices)?;
            print!("{}", render_reply(&r));
            Ok(())
        }
        AgentAction::Snapshot { path } => {
            let mut c = connect()?;
            let device = select(&mut c, pid, path)?;
            let r = call(&mut c, Request::ReadSnapshot { device })?;
            print!("{}", render_reply(&r));
            Ok(())
        }
        AgentAction::Apply {
            file,
            apply,
            all,
            force,
            path,
        } => apply_profile(&file, apply, all, force, pid, path),
        AgentAction::AppProfiles { action } => {
            let req = match action {
                Some(AppProfilesAction::Reload) => Request::ReloadAppProfiles,
                None => Request::AppProfiles,
            };
            let r = call(&mut connect()?, req)?;
            print!("{}", render_reply(&r));
            match r {
                Reply::AppProfiles(AppProfilesInfo { error: Some(e), .. }) if action.is_some() => {
                    bail!("the rules were not loaded: {e}")
                }
                _ => Ok(()),
            }
        }
        AgentAction::Stop => {
            let r = call(&mut connect()?, Request::Shutdown)?;
            println!("{}", render_reply(&r));
            Ok(())
        }
    }
}

/// The device from the flags, or the only one the agent sees.
fn select(c: &mut Client, pid: Option<u16>, path: Option<String>) -> Result<DeviceRef> {
    Ok(match device_ref(pid, path) {
        Some(d) => d,
        None => match call(c, Request::ListDevices)? {
            Reply::Devices { list, .. } => pick_device(&list).map_err(|e| anyhow!(e))?,
            other => bail!("unexpected reply: {}", render_reply(&other)),
        },
    })
}

/// `agent apply`: send the profile text, print the agent's lines, save the restore
/// profile next to `file` (same naming as `profile apply`).
fn apply_profile(
    file: &Path,
    apply: bool,
    all: bool,
    force: bool,
    pid: Option<u16>,
    path: Option<String>,
) -> Result<()> {
    // Parsed here too, so a broken file is reported with its name before connecting.
    let (toml, _) = crate::profile::read_profile(file)?;
    let mut c = connect()?;
    let device = select(&mut c, pid, path)?;
    let req = Request::ApplyProfile {
        device: device.clone(),
        toml,
        dry_run: !apply,
        all,
        opts: WireWriteOpts {
            confirmed: force,
            ..WireWriteOpts::default()
        },
    };
    let Reply::ProfileApplied {
        lines,
        restore_toml,
        error,
    } = call(&mut c, req)?
    else {
        bail!("unexpected reply to ApplyProfile");
    };
    let restore = crate::profile::restore_path(file, Path::exists);
    let saved = match (&restore_toml, apply) {
        (Some(text), true) => {
            std::fs::write(&restore, text)
                .with_context(|| format!("cannot write the restore file {}", restore.display()))?;
            true
        }
        _ => false,
    };
    print!(
        "{}",
        render_applied(
            &lines,
            restore_toml.is_some().then_some(restore.as_path()),
            saved,
            &device
        )
    );
    match error {
        Some(e) => bail!(e),
        None => Ok(()),
    }
}

/// The agent's lines, then where the restore profile went (or would go) and the
/// command that applies it.
pub(crate) fn render_applied(
    lines: &[String],
    restore: Option<&Path>,
    saved: bool,
    device: &DeviceRef,
) -> String {
    let mut s = String::new();
    for l in lines {
        let _ = writeln!(s, "{l}");
    }
    match (restore, saved) {
        (Some(r), true) => {
            let sel = match device {
                DeviceRef::Pid(p) => format!("--pid {p:04X} "),
                DeviceRef::Path(p) => format!("--path \"{p}\" "),
            };
            let _ = writeln!(s, "Saved the values from before to {}.", r.display());
            let _ = writeln!(
                s,
                "To restore: opensynapse agent apply {sel}{} --apply",
                r.display()
            );
        }
        (Some(r), false) => {
            let _ = writeln!(
                s,
                "With --apply, the current values are saved to {} first.",
                r.display()
            );
        }
        (None, _) => {}
    }
    s
}

/// The agent's pipe: `OPENSYNAPSE_AGENT_PIPE`, else the current user's default.
fn pipe_name() -> Result<String> {
    match std::env::var(PIPE_ENV) {
        Ok(n) if !n.trim().is_empty() => Ok(n),
        _ => default_pipe_name().map_err(|e| anyhow!(render_io_error(&e))),
    }
}

fn connect() -> Result<Client> {
    Client::connect_to(&pipe_name()?).map_err(|e| anyhow!(render_io_error(&e)))
}

fn call(c: &mut Client, req: Request) -> Result<Reply> {
    match c.call(req) {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => bail!(render_ipc_error(&e)),
        Err(e) => bail!(render_io_error(&e)),
    }
}

/// Block on the subscription and print each event. No timers: the iterator only
/// returns when the agent sends something or closes the pipe.
fn watch() -> Result<()> {
    let name = pipe_name()?;
    let sub = Client::subscribe_to(&name).map_err(|e| anyhow!(render_io_error(&e)))?;
    println!("Watching agent events (Ctrl+C to stop).");
    for ev in sub {
        let ev = ev.map_err(|e| anyhow!(render_io_error(&e)))?;
        println!("{}", render_event(&ev));
        if ev == Event::ShuttingDown {
            break;
        }
    }
    Ok(())
}

/// The device a `snapshot` targets from the CLI flags, or `None` to pick automatically.
pub(crate) fn device_ref(pid: Option<u16>, path: Option<String>) -> Option<DeviceRef> {
    match (path, pid) {
        (Some(p), _) => Some(DeviceRef::Path(p)),
        (None, Some(pid)) => Some(DeviceRef::Pid(pid)),
        (None, None) => None,
    }
}

/// The only device in `list`, or why there is no single one.
pub(crate) fn pick_device(list: &[DeviceInfo]) -> Result<DeviceRef, String> {
    match list {
        [d] => Ok(DeviceRef::Path(d.path.clone())),
        [] => Err("the agent sees no Razer devices".into()),
        _ => Err(format!(
            "the agent sees {} devices; choose one with --pid or --path (see `opensynapse agent devices`)",
            list.len()
        )),
    }
}

/// Message for a transport error; a missing pipe means the agent is not running.
pub(crate) fn render_io_error(e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::NotFound {
        "OpenSynapse agent is not running (start it with `opensynapse agent start`)".into()
    } else {
        format!("agent connection failed: {e}")
    }
}

/// Message for an error the agent returned.
pub(crate) fn render_ipc_error(e: &IpcError) -> String {
    let kind = match e.kind {
        ErrorKind::BadRequest => "bad request",
        ErrorKind::Version => "protocol version mismatch",
        ErrorKind::NoSuchDevice => "no such device",
        ErrorKind::Ambiguous => "ambiguous device",
        ErrorKind::Unsupported => "unsupported",
        ErrorKind::Conflict => "conflict",
        ErrorKind::Device => "device error",
    };
    format!("agent: {kind}: {}", e.message)
}

/// Output for any reply; multi-line replies end with a newline.
pub(crate) fn render_reply(r: &Reply) -> String {
    match r {
        Reply::Pong { agent_version } => render_pong(agent_version),
        Reply::Devices { list, conflict } => render_devices(list, conflict.as_deref()),
        Reply::Snapshot(s) => render_snapshot(s),
        Reply::Written(Ok(w)) => format!(
            "{} -> {}, now {} ({})",
            render_setting(&w.old),
            render_setting(&w.new),
            render_setting(&w.now),
            if w.verified {
                "verified"
            } else {
                "NOT verified"
            }
        ),
        Reply::Written(Err(f)) => match &f.old {
            Some(old) => format!(
                "write failed: {} (old value: {})",
                f.message,
                render_setting(old)
            ),
            None => format!("write failed: {}", f.message),
        },
        Reply::ProfileApplied { lines, .. } => {
            let mut s = String::new();
            for l in lines {
                let _ = writeln!(s, "{l}");
            }
            s
        }
        Reply::ShuttingDown => "Agent is shutting down.".into(),
        Reply::AppProfiles(i) => render_app_profiles(i),
    }
}

/// `agent app-profiles [reload]`: file, state, rules, what is active.
pub(crate) fn render_app_profiles(i: &AppProfilesInfo) -> String {
    let mut s = String::new();
    let file = i.config.as_deref().unwrap_or("(no APPDATA folder)");
    let _ = writeln!(s, "Rules file: {file}");
    match (&i.error, i.enabled) {
        (Some(e), _) => {
            let _ = writeln!(s, "Per-app profiles: off (invalid file: {e})");
        }
        (None, false) => {
            let _ = writeln!(s, "Per-app profiles: off (no rules file)");
        }
        (None, true) => {
            let _ = writeln!(s, "Per-app profiles: on, {} rule(s)", i.rules.len());
        }
    }
    for r in &i.rules {
        let _ = writeln!(s, "  {} -> {}", r.exe, r.profile);
    }
    if let Some(d) = &i.default {
        let _ = writeln!(s, "  (no match) -> {d}");
    }
    let _ = writeln!(
        s,
        "Foreground: {}",
        i.foreground.as_deref().unwrap_or("(unknown)")
    );
    let _ = writeln!(s, "Active: {}", i.active.as_deref().unwrap_or("(none)"));
    if i.restore_pending {
        let _ = writeln!(
            s,
            "Values from before the first app profile are kept for restoring."
        );
    }
    s
}

pub(crate) fn render_pong(agent_version: &str) -> String {
    format!("Agent running (version {agent_version}).")
}

/// One line per device, like `opensynapse list`, plus the path for `--path`.
pub(crate) fn render_devices(list: &[DeviceInfo], conflict: Option<&str>) -> String {
    let mut s = String::new();
    if list.is_empty() {
        s.push_str("No Razer devices found.\n");
    }
    for d in list {
        let _ = writeln!(s, "{}", device_line(d));
    }
    if let Some(c) = conflict {
        let _ = writeln!(
            s,
            "Note: Razer software is running ({c}); writes need confirmation."
        );
    }
    s
}

fn device_line(d: &DeviceInfo) -> String {
    let name = d.name.as_deref().unwrap_or("unknown Razer device");
    let sup = if d.supported {
        "supported"
    } else {
        "unsupported"
    };
    format!(
        "{:04X}  {name}  (protocol {}, {sup})  {}",
        d.pid, d.protocol, d.path
    )
}

/// All snapshot fields in the style of `info`, `battery`, `dpi-stages`, … .
pub(crate) fn render_snapshot(s: &WireSnapshot) -> String {
    let mut o = String::new();
    field(&mut o, "Firmware", &s.firmware, |v| v.to_string());
    field(&mut o, "Serial", &s.serial, |v| v.clone());
    field(&mut o, "Device mode", &s.mode, |(m, p)| {
        format!("{m:?} (param {p})")
    });
    field(&mut o, "Battery", &s.battery, |b| {
        format!("{}% (raw {})", b.percent(), b.0)
    });
    field(&mut o, "Charging", &s.charging, |c| c.to_string());
    field(&mut o, "DPI", &s.dpi, |d| d.to_string());
    match &s.stages {
        Ok(st) => {
            o.push_str("DPI stages:\n");
            for (i, stage) in st.stages.iter().enumerate() {
                let mark = if stage.id == st.active { "*" } else { " " };
                let pos = if usize::from(stage.id) == i + 1 {
                    String::new()
                } else {
                    format!(" (position {})", i + 1)
                };
                let _ = writeln!(o, "{mark} stage {}: {}{pos}", stage.id, stage.dpi);
            }
        }
        Err(e) => {
            let _ = writeln!(o, "DPI stages: n/a ({e})");
        }
    }
    field(&mut o, "Polling rate", &s.polling, |(hz, v)| {
        format!("{hz} Hz ({})", variant_name(*v))
    });
    field(&mut o, "Sleep timer", &s.sleep, |v| format!("{v} s"));
    buttons(&mut o, "Buttons (normal)", &s.buttons_normal);
    buttons(&mut o, "Buttons (HyperShift)", &s.buttons_hypershift);
    o
}

fn field<T>(o: &mut String, label: &str, f: &Field<T>, show: impl Fn(&T) -> String) {
    let _ = match f {
        Ok(v) => writeln!(o, "{label}: {}", show(v)),
        Err(e) => writeln!(o, "{label}: n/a ({e})"),
    };
}

fn buttons(
    o: &mut String,
    label: &str,
    f: &Field<Vec<opensynapse_protocol::buttons::ButtonAssignment>>,
) {
    match f {
        Ok(list) if list.is_empty() => {
            let _ = writeln!(o, "{label}: none");
        }
        Ok(list) => {
            let _ = writeln!(o, "{label}:");
            for b in list {
                let _ = writeln!(o, "  {b}");
            }
        }
        Err(e) => {
            let _ = writeln!(o, "{label}: n/a ({e})");
        }
    }
}

fn variant_name(v: PollingVariant) -> &'static str {
    match v {
        PollingVariant::Period => "period",
        PollingVariant::Hyper => "hyper",
    }
}

fn render_setting(s: &WireSetting) -> String {
    match s {
        WireSetting::Dpi(d) => format!("DPI {d}"),
        WireSetting::Polling { hz, variant } => {
            format!("polling {hz} Hz ({})", variant_name(*variant))
        }
        WireSetting::Sleep(v) => format!("sleep timer {v} s"),
        WireSetting::Button(b) => b.to_string(),
        WireSetting::DpiStages { storage, stages } => {
            let list: Vec<String> = stages
                .stages
                .iter()
                .map(|st| {
                    let mark = if st.id == stages.active { "*" } else { "" };
                    format!("{mark}{}", st.dpi)
                })
                .collect();
            format!("DPI stages ({storage:?}) [{}]", list.join(", "))
        }
    }
}

/// One line per event.
pub(crate) fn render_event(e: &Event) -> String {
    match e {
        Event::DeviceArrived(d) => format!("arrived: {}", device_line(d)),
        Event::DeviceRemoved { path, pid } => format!("removed: {pid:04X}  {path}"),
        Event::Battery {
            path,
            level,
            charging,
        } => format!(
            "battery: {}% (raw {level}), {}  {path}",
            BatteryLevel(*level).percent(),
            ChargingStatus::from_byte(*charging)
        ),
        Event::DpiChanged { path, x, y } => {
            let d = opensynapse_protocol::dpi::Dpi { x: *x, y: *y };
            format!("DPI: {d}  {path}")
        }
        Event::Hardware { path, id, payload } => {
            let hex: Vec<String> = payload.iter().map(|b| format!("{b:02X}")).collect();
            format!("hardware event {id:#04x} [{}]  {path}", hex.join(" "))
        }
        Event::Error { message } => format!("agent error: {message}"),
        Event::ShuttingDown => "Agent is shutting down.".into(),
        Event::AppProfile {
            exe,
            profile,
            lines,
            error,
        } => {
            let what = match profile {
                Some(p) => format!("app profile {p}"),
                None => "values from before restored".into(),
            };
            let mut s = match error {
                Some(e) => format!("{what} for {exe} FAILED: {e}"),
                None => format!("{what} for {exe}"),
            };
            for l in lines {
                s.push_str("\n  ");
                s.push_str(l);
            }
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Command};
    use clap::Parser;
    use opensynapse_ipc::msg::{WriteDone, WriteFailed};
    use opensynapse_protocol::buttons::{ButtonAssignment, Mode};
    use opensynapse_protocol::device::{DeviceMode, FirmwareVersion};
    use opensynapse_protocol::dpi::{Dpi, DpiStage, DpiStages, Storage};

    fn parse(args: &[&str]) -> (AgentAction, Option<u16>) {
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Agent { action } => (action, cli.pid),
            _ => panic!("not an agent command"),
        }
    }

    #[test]
    fn parses_subcommands() {
        assert_eq!(parse(&["os", "agent", "ping"]).0, AgentAction::Ping);
        assert_eq!(parse(&["os", "agent", "devices"]).0, AgentAction::Devices);
        assert_eq!(parse(&["os", "agent", "watch"]).0, AgentAction::Watch);
        assert_eq!(parse(&["os", "agent", "stop"]).0, AgentAction::Stop);
        assert_eq!(parse(&["os", "agent", "start"]).0, AgentAction::Start);
        assert_eq!(
            parse(&["os", "agent", "autostart"]).0,
            AgentAction::Autostart { state: None }
        );
        assert_eq!(
            parse(&["os", "agent", "autostart", "on"]).0,
            AgentAction::Autostart {
                state: Some(crate::lifecycle::Toggle::On)
            }
        );
        assert!(Cli::try_parse_from(["os", "agent", "autostart", "maybe"]).is_err());
        assert_eq!(
            parse(&["os", "agent", "app-profiles"]).0,
            AgentAction::AppProfiles { action: None }
        );
        assert_eq!(
            parse(&["os", "agent", "app-profiles", "reload"]).0,
            AgentAction::AppProfiles {
                action: Some(AppProfilesAction::Reload)
            }
        );
        assert_eq!(
            parse(&["os", "agent", "snapshot"]),
            (AgentAction::Snapshot { path: None }, None)
        );
    }

    #[test]
    fn snapshot_selects_by_pid_or_path() {
        let (a, pid) = parse(&["os", "agent", "snapshot", "--pid", "00B8"]);
        assert_eq!(pid, Some(0x00B8));
        assert_eq!(a, AgentAction::Snapshot { path: None });
        assert_eq!(device_ref(pid, None), Some(DeviceRef::Pid(0x00B8)));

        let (a, pid) = parse(&["os", "--pid", "b8", "agent", "snapshot"]);
        assert_eq!((a, pid), (AgentAction::Snapshot { path: None }, Some(0xB8)));

        let (a, pid) = parse(&["os", "agent", "snapshot", "--path", "\\\\?\\hid#x"]);
        let AgentAction::Snapshot { path } = a else {
            panic!()
        };
        assert_eq!(
            device_ref(pid, path),
            Some(DeviceRef::Path("\\\\?\\hid#x".into()))
        );
        assert_eq!(device_ref(None, None), None);
    }

    #[test]
    fn pid_and_path_conflict() {
        let r = Cli::try_parse_from(["os", "agent", "snapshot", "--pid", "00B8", "--path", "p"]);
        assert_eq!(
            r.err().map(|e| e.kind()),
            Some(clap::error::ErrorKind::ArgumentConflict)
        );
    }

    fn dev(pid: u16, path: &str) -> DeviceInfo {
        DeviceInfo {
            path: path.into(),
            pid,
            name: Some("Razer Viper V3 HyperSpeed".into()),
            protocol: "2.5".into(),
            supported: true,
        }
    }

    #[test]
    fn picks_the_only_device() {
        assert_eq!(
            pick_device(&[dev(0xB8, "p1")]),
            Ok(DeviceRef::Path("p1".into()))
        );
        assert!(pick_device(&[]).unwrap_err().contains("no Razer devices"));
        assert!(
            pick_device(&[dev(0xB8, "p1"), dev(0xB9, "p2")])
                .unwrap_err()
                .contains("--pid or --path")
        );
    }

    #[test]
    fn not_found_means_agent_not_running() {
        let e = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(
            render_io_error(&e),
            "OpenSynapse agent is not running (start it with `opensynapse agent start`)"
        );
        let e = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(render_io_error(&e).starts_with("agent connection failed:"));
    }

    #[test]
    fn ipc_error() {
        let e = IpcError {
            kind: ErrorKind::NoSuchDevice,
            message: "no device with PID 00B9".into(),
        };
        assert_eq!(
            render_ipc_error(&e),
            "agent: no such device: no device with PID 00B9"
        );
    }

    #[test]
    fn pong() {
        let r = Reply::Pong {
            agent_version: "0.1.0".into(),
        };
        assert_eq!(render_reply(&r), "Agent running (version 0.1.0).");
    }

    #[test]
    fn devices() {
        let mut unknown = dev(0x1234, "p2");
        unknown.name = None;
        unknown.supported = false;
        unknown.protocol = "unknown".into();
        let r = Reply::Devices {
            list: vec![dev(0xB8, "p1"), unknown],
            conflict: Some("RazerAppEngine.exe".into()),
        };
        assert_eq!(
            render_reply(&r),
            "00B8  Razer Viper V3 HyperSpeed  (protocol 2.5, supported)  p1\n\
             1234  unknown Razer device  (protocol unknown, unsupported)  p2\n\
             Note: Razer software is running (RazerAppEngine.exe); writes need confirmation.\n"
        );
        assert_eq!(render_devices(&[], None), "No Razer devices found.\n");
    }

    fn button(button: u8, mode: Mode) -> ButtonAssignment {
        ButtonAssignment {
            profile: 1,
            button,
            mode,
            function: 1,
            data: vec![button],
        }
    }

    fn stages() -> DpiStages {
        DpiStages {
            active: 2,
            stages: vec![
                DpiStage {
                    id: 1,
                    dpi: Dpi::uniform(400),
                },
                DpiStage {
                    id: 2,
                    dpi: Dpi::uniform(800),
                },
            ],
        }
    }

    #[test]
    fn snapshot() {
        let s = WireSnapshot {
            firmware: Ok(FirmwareVersion { major: 1, minor: 2 }),
            serial: Ok("PM123".into()),
            mode: Ok((DeviceMode::Normal, 0)),
            battery: Ok(BatteryLevel(255)),
            charging: Ok(ChargingStatus::NotCharging),
            dpi: Ok(Dpi::uniform(800)),
            stages: Ok(stages()),
            polling: Ok((1000, PollingVariant::Period)),
            sleep: Err("timeout".into()),
            buttons_normal: Ok(vec![button(4, Mode::Normal)]),
            buttons_hypershift: Ok(vec![]),
        };
        assert_eq!(
            render_reply(&Reply::Snapshot(s)),
            "Firmware: 1.02\n\
             Serial: PM123\n\
             Device mode: Normal (param 0)\n\
             Battery: 100% (raw 255)\n\
             Charging: not charging\n\
             DPI: 800\n\
             DPI stages:\n\
             \x20 stage 1: 400\n\
             * stage 2: 800\n\
             Polling rate: 1000 Hz (period)\n\
             Sleep timer: n/a (timeout)\n\
             Buttons (normal):\n\
             \x20 profile 1, button 4, normal mode: mouse button 4 (function 1, data [4])\n\
             Buttons (HyperShift): none\n"
        );
    }

    #[test]
    fn snapshot_errors() {
        fn e<T>() -> Field<T> {
            Err("busy".to_string())
        }
        let s = WireSnapshot {
            firmware: e(),
            serial: e(),
            mode: e(),
            battery: e(),
            charging: e(),
            dpi: e(),
            stages: e(),
            polling: e(),
            sleep: e(),
            buttons_normal: e(),
            buttons_hypershift: e(),
        };
        let out = render_snapshot(&s);
        assert_eq!(out.lines().count(), 11);
        assert!(out.lines().all(|l| l.ends_with("n/a (busy)")), "{out}");
    }

    #[test]
    fn written() {
        let ok = Reply::Written(Ok(WriteDone {
            old: WireSetting::Dpi(Dpi::uniform(800)),
            new: WireSetting::Dpi(Dpi::uniform(1600)),
            now: WireSetting::Dpi(Dpi::uniform(1600)),
            verified: true,
        }));
        assert_eq!(
            render_reply(&ok),
            "DPI 800 -> DPI 1600, now DPI 1600 (verified)"
        );
        let failed = Reply::Written(Err(WriteFailed {
            old: Some(WireSetting::Sleep(300)),
            message: "device busy".into(),
        }));
        assert_eq!(
            render_reply(&failed),
            "write failed: device busy (old value: sleep timer 300 s)"
        );
        let failed = Reply::Written(Err(WriteFailed {
            old: None,
            message: "x".into(),
        }));
        assert_eq!(render_reply(&failed), "write failed: x");
    }

    #[test]
    fn settings() {
        assert_eq!(
            render_setting(&WireSetting::Polling {
                hz: 1000,
                variant: PollingVariant::Hyper
            }),
            "polling 1000 Hz (hyper)"
        );
        assert_eq!(
            render_setting(&WireSetting::Button(button(5, Mode::HyperShift))),
            "profile 1, button 5, HyperShift mode: mouse button 5 (function 1, data [5])"
        );
        assert_eq!(
            render_setting(&WireSetting::DpiStages {
                storage: Storage::Persistent,
                stages: stages()
            }),
            "DPI stages (Persistent) [400, *800]"
        );
    }

    #[test]
    fn profile_applied_and_shutdown() {
        let r = Reply::ProfileApplied {
            lines: vec!["a".into(), "b".into()],
            restore_toml: Some("x".into()),
            error: None,
        };
        assert_eq!(render_reply(&r), "a\nb\n");
        assert_eq!(
            render_reply(&Reply::ShuttingDown),
            "Agent is shutting down."
        );
    }

    #[test]
    fn parses_apply() {
        let (a, pid) = parse(&["os", "agent", "apply", "g.toml"]);
        assert_eq!(
            (a, pid),
            (
                AgentAction::Apply {
                    file: "g.toml".into(),
                    apply: false,
                    all: false,
                    force: false,
                    path: None
                },
                None
            )
        );
        let (a, pid) = parse(&[
            "os", "--pid", "00B8", "agent", "apply", "g.toml", "--apply", "--all", "--force",
        ]);
        assert_eq!(pid, Some(0x00B8));
        assert_eq!(
            a,
            AgentAction::Apply {
                file: "g.toml".into(),
                apply: true,
                all: true,
                force: true,
                path: None
            }
        );
        let r = Cli::try_parse_from(["os", "agent", "apply", "g", "--pid", "1", "--path", "p"]);
        assert_eq!(
            r.err().map(|e| e.kind()),
            Some(clap::error::ErrorKind::ArgumentConflict)
        );
    }

    #[test]
    fn applied_output() {
        let lines = vec!["1 write(s), 0 unchanged.".to_string()];
        let r = Path::new("dir/g.restore.toml");
        assert_eq!(
            render_applied(&lines, Some(r), true, &DeviceRef::Pid(0xB8)),
            format!(
                "1 write(s), 0 unchanged.\n\
                 Saved the values from before to {0}.\n\
                 To restore: opensynapse agent apply --pid 00B8 {0} --apply\n",
                r.display()
            )
        );
        assert_eq!(
            render_applied(&[], Some(r), false, &DeviceRef::Pid(0xB8)),
            format!(
                "With --apply, the current values are saved to {} first.\n",
                r.display()
            )
        );
        assert!(
            render_applied(&[], Some(r), true, &DeviceRef::Path("p".into()))
                .contains("agent apply --path \"p\" ")
        );
        assert_eq!(
            render_applied(&lines, None, false, &DeviceRef::Pid(0xB8)),
            "1 write(s), 0 unchanged.\n"
        );
    }

    #[test]
    fn events() {
        assert_eq!(
            render_event(&Event::DeviceArrived(dev(0xB8, "p1"))),
            "arrived: 00B8  Razer Viper V3 HyperSpeed  (protocol 2.5, supported)  p1"
        );
        assert_eq!(
            render_event(&Event::DeviceRemoved {
                path: "p1".into(),
                pid: 0xB8
            }),
            "removed: 00B8  p1"
        );
        assert_eq!(
            render_event(&Event::Battery {
                path: "p1".into(),
                level: 128,
                charging: 1
            }),
            "battery: 50% (raw 128), charging  p1"
        );
        assert_eq!(
            render_event(&Event::DpiChanged {
                path: "p1".into(),
                x: 800,
                y: 1600
            }),
            "DPI: 800x1600  p1"
        );
        assert_eq!(
            render_event(&Event::Hardware {
                path: "p1".into(),
                id: 0x31,
                payload: vec![0x01, 0xAB]
            }),
            "hardware event 0x31 [01 AB]  p1"
        );
        assert_eq!(
            render_event(&Event::Error {
                message: "boom".into()
            }),
            "agent error: boom"
        );
        assert_eq!(
            render_event(&Event::ShuttingDown),
            "Agent is shutting down."
        );
        assert_eq!(
            render_event(&Event::AppProfile {
                exe: "C:\\cs2.exe".into(),
                profile: Some("p/cs2.toml".into()),
                lines: vec!["1 write(s), 0 unchanged.".into()],
                error: None,
            }),
            "app profile p/cs2.toml for C:\\cs2.exe\n  1 write(s), 0 unchanged."
        );
        assert_eq!(
            render_event(&Event::AppProfile {
                exe: "x.exe".into(),
                profile: None,
                lines: vec![],
                error: Some("busy".into()),
            }),
            "values from before restored for x.exe FAILED: busy"
        );
    }

    #[test]
    fn app_profiles() {
        use opensynapse_ipc::msg::AppRuleInfo;
        assert_eq!(
            render_app_profiles(&AppProfilesInfo::default()),
            "Rules file: (no APPDATA folder)\nPer-app profiles: off (no rules file)\n\
             Foreground: (unknown)\nActive: (none)\n"
        );
        let i = AppProfilesInfo {
            config: Some("r.toml".into()),
            enabled: true,
            error: None,
            default: Some("d.toml".into()),
            rules: vec![AppRuleInfo {
                exe: "cs2.exe".into(),
                profile: "c.toml".into(),
            }],
            foreground: Some("cs2.exe".into()),
            active: Some("c.toml".into()),
            restore_pending: true,
        };
        assert_eq!(
            render_reply(&Reply::AppProfiles(i)),
            "Rules file: r.toml\nPer-app profiles: on, 1 rule(s)\n  cs2.exe -> c.toml\n\
             \x20 (no match) -> d.toml\nForeground: cs2.exe\nActive: c.toml\n\
             Values from before the first app profile are kept for restoring.\n"
        );
        let bad = AppProfilesInfo {
            error: Some("rule 1: `exe` must not be empty".into()),
            ..AppProfilesInfo::default()
        };
        assert!(render_app_profiles(&bad).contains("off (invalid file: rule 1"));
    }
}
