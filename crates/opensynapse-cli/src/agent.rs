//! `opensynapse agent …`: talk to a running `opensynapse-agent` over local IPC
//! (`vault/Decisions/ADR-0005 Agent IPC.md`). Read-only in slice 1, plus `stop`.
//!
//! All output is built by the pure `render_*` functions so it can be tested without
//! a running agent; [`run`] only connects, calls and prints.

use std::fmt::Write as _;
use std::io;

use anyhow::{Result, anyhow, bail};
use clap::Subcommand;
use opensynapse_ipc::Client;
use opensynapse_ipc::msg::{
    DeviceInfo, DeviceRef, ErrorKind, Event, Field, IpcError, PollingVariant, Reply, Request,
    WireSetting, WireSnapshot,
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
    /// Print agent events (hotplug, battery, …) as they arrive, until the agent stops.
    Watch,
    /// Ask the agent to shut down.
    Stop,
}

/// Run one agent command. `pid` is the global `--pid`.
pub fn run(action: AgentAction, pid: Option<u16>) -> Result<()> {
    match action {
        AgentAction::Watch => watch(),
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
            let device = match device_ref(pid, path) {
                Some(d) => d,
                None => match call(&mut c, Request::ListDevices)? {
                    Reply::Devices { list, .. } => pick_device(&list).map_err(|e| anyhow!(e))?,
                    other => bail!("unexpected reply: {}", render_reply(&other)),
                },
            };
            let r = call(&mut c, Request::ReadSnapshot { device })?;
            print!("{}", render_reply(&r));
            Ok(())
        }
        AgentAction::Stop => {
            let r = call(&mut connect()?, Request::Shutdown)?;
            println!("{}", render_reply(&r));
            Ok(())
        }
    }
}

fn connect() -> Result<Client> {
    Client::connect().map_err(|e| anyhow!(render_io_error(&e)))
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
    let name = default_pipe_name().map_err(|e| anyhow!(render_io_error(&e)))?;
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
        "OpenSynapse agent is not running (start opensynapse-agent)".into()
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
        Reply::ProfileApplied { lines } => {
            let mut s = String::new();
            for l in lines {
                let _ = writeln!(s, "{l}");
            }
            s
        }
        Reply::ShuttingDown => "Agent is shutting down.".into(),
    }
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
            "OpenSynapse agent is not running (start opensynapse-agent)"
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
        };
        assert_eq!(render_reply(&r), "a\nb\n");
        assert_eq!(
            render_reply(&Reply::ShuttingDown),
            "Agent is shutting down."
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
    }
}
