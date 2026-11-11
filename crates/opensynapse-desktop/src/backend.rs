//! Which backend serves the GUI, and how agent events change the panel. Pure
//! functions; the threads live in [`crate::worker`].

use std::io;

use opensynapse_ipc::msg::Event;
use opensynapse_protocol::dpi::Dpi;
use opensynapse_protocol::power::{BatteryLevel, ChargingStatus};

use crate::ops::Snapshot;

/// What the top bar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendState {
    /// Requests go to the background agent over IPC.
    Agent { pid: u32 },
    /// In-process HID on the worker thread. `note` says why, if the agent was
    /// there but could not be used (or went away).
    Local { note: Option<String> },
}

impl BackendState {
    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }

    pub fn label(&self) -> String {
        match self {
            Self::Agent { pid } => format!("Backend: agent (pid {pid})"),
            Self::Local { .. } => "Backend: direct HID".into(),
        }
    }

    pub fn note(&self) -> Option<&str> {
        match self {
            Self::Agent { .. } => None,
            Self::Local { note } => note.as_deref(),
        }
    }
}

/// Outcome of a connection attempt: keep the client and use the agent, or fall
/// back to direct HID. No agent running (`NotFound`) is the normal case and
/// needs no note.
pub fn select<C>(attempt: io::Result<C>, pid: impl FnOnce(&C) -> u32) -> (Option<C>, BackendState) {
    match attempt {
        Ok(c) => {
            let pid = pid(&c);
            (Some(c), BackendState::Agent { pid })
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (None, BackendState::Local { note: None }),
        Err(e) => (
            None,
            BackendState::Local {
                note: Some(format!("agent not usable ({e}); using direct HID")),
            },
        ),
    }
}

/// The local state after the agent went away.
pub fn lost(reason: Option<&str>) -> BackendState {
    let note = match reason {
        Some(r) => format!("lost the agent ({r}); using direct HID"),
        None => "agent stopped; using direct HID".into(),
    };
    BackendState::Local { note: Some(note) }
}

/// A backend switch serves a different device set: list devices again. The
/// first selection (`prev == None`) comes with its own scan.
pub fn needs_rescan(prev: Option<&BackendState>, now: &BackendState) -> bool {
    prev.is_some_and(|p| p.is_agent() != now.is_agent())
}

/// What the app does after an agent event.
#[derive(Debug, PartialEq, Eq)]
pub enum EventEffect {
    /// Nothing visible changed.
    Nothing,
    /// A device came or went: list devices again.
    Rescan,
    /// The shown snapshot was updated in place.
    Updated,
    /// The agent wrote to devices (per-app profile switch, also for a device
    /// plugged in mid-session): read the selected device again.
    Reread,
    /// Show this message.
    Note(String),
}

/// Apply one event to the snapshot of the selected device.
pub fn apply_event(
    selected: Option<&str>,
    snapshot: &mut Option<Result<Snapshot, String>>,
    ev: Event,
) -> EventEffect {
    let shown = |path: &str| selected == Some(path);
    match ev {
        Event::DeviceArrived(_) | Event::DeviceRemoved { .. } => EventEffect::Rescan,
        Event::Battery {
            path,
            level,
            charging,
        } => match snapshot {
            Some(Ok(s)) if shown(&path) => {
                s.battery = Ok(BatteryLevel(level));
                // Event byte: 0 no / 1 yes / 2 fault; same values as the
                // charging-status read (inferred).
                s.charging = Ok(ChargingStatus::from_byte(charging));
                EventEffect::Updated
            }
            _ => EventEffect::Nothing,
        },
        Event::DpiChanged { path, x, y } => match snapshot {
            Some(Ok(s)) if shown(&path) => {
                s.dpi = Ok(Dpi { x, y });
                EventEffect::Updated
            }
            _ => EventEffect::Nothing,
        },
        Event::Error { message } => EventEffect::Note(format!("agent: {message}")),
        // The subscription ends right after; that switches the backend.
        // An app profile switch may write any setting without a device event
        // (e.g. DPI stages on a device that just arrived), so read it again.
        Event::AppProfile { .. } if selected.is_some() => EventEffect::Reread,
        Event::ShuttingDown | Event::Hardware { .. } | Event::AppProfile { .. } => {
            EventEffect::Nothing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensynapse_ipc::msg::DeviceInfo;

    fn snapshot() -> Snapshot {
        let w = crate::remote::tests::wire_snapshot();
        crate::remote::snapshot_from_wire(w).unwrap()
    }

    #[test]
    fn select_agent_or_local() {
        let (c, s) = select(Ok(7u32), |c| *c * 2);
        assert_eq!((c, &s), (Some(7), &BackendState::Agent { pid: 14 }));
        assert_eq!(s.label(), "Backend: agent (pid 14)");
        assert!(s.is_agent());

        let (c, s) = select::<u32>(Err(io::ErrorKind::NotFound.into()), |_| 0);
        assert_eq!((c, &s), (None, &BackendState::Local { note: None }));
        assert_eq!(s.label(), "Backend: direct HID");

        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "other user");
        let (c, s) = select::<u32>(Err(denied), |_| 0);
        assert!(c.is_none() && !s.is_agent());
        assert!(s.note().unwrap().contains("other user"));
    }

    #[test]
    fn rescan_only_on_a_switch() {
        let agent = BackendState::Agent { pid: 1 };
        let local = BackendState::Local { note: None };
        assert!(!needs_rescan(None, &agent));
        assert!(!needs_rescan(None, &local));
        assert!(needs_rescan(Some(&agent), &lost(None)));
        assert!(needs_rescan(Some(&local), &agent));
        assert!(!needs_rescan(Some(&local), &lost(Some("x"))));
        assert!(!needs_rescan(Some(&agent), &agent));
    }

    #[test]
    fn lost_notes() {
        assert_eq!(lost(None).note(), Some("agent stopped; using direct HID"));
        assert!(
            lost(Some("broken pipe"))
                .note()
                .unwrap()
                .contains("broken pipe")
        );
    }

    #[test]
    fn hotplug_rescans() {
        let mut snap = None;
        let info = DeviceInfo {
            path: "p".into(),
            pid: 0xB8,
            name: None,
            protocol: "2.5".into(),
            supported: true,
        };
        assert_eq!(
            apply_event(None, &mut snap, Event::DeviceArrived(info)),
            EventEffect::Rescan
        );
        let gone = Event::DeviceRemoved {
            path: "p".into(),
            pid: 0xB8,
        };
        assert_eq!(apply_event(Some("p"), &mut snap, gone), EventEffect::Rescan);
    }

    #[test]
    fn battery_and_dpi_update_the_selected_snapshot() {
        let mut snap = Some(Ok(snapshot()));
        let bat = |path: &str| Event::Battery {
            path: path.into(),
            level: 255,
            charging: 1,
        };
        assert_eq!(
            apply_event(Some("p"), &mut snap, bat("other")),
            EventEffect::Nothing
        );
        assert_eq!(
            apply_event(Some("p"), &mut snap, bat("p")),
            EventEffect::Updated
        );
        let s = snap.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(s.battery, Ok(BatteryLevel(255)));
        assert_eq!(s.charging, Ok(ChargingStatus::Charging));

        let dpi = Event::DpiChanged {
            path: "p".into(),
            x: 1600,
            y: 800,
        };
        assert_eq!(apply_event(Some("p"), &mut snap, dpi), EventEffect::Updated);
        let s = snap.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(s.dpi, Ok(Dpi { x: 1600, y: 800 }));
    }

    #[test]
    fn events_without_a_snapshot_change_nothing() {
        let dpi = || Event::DpiChanged {
            path: "p".into(),
            x: 1,
            y: 1,
        };
        let mut none = None;
        assert_eq!(
            apply_event(Some("p"), &mut none, dpi()),
            EventEffect::Nothing
        );
        let mut failed = Some(Err("n/a".into()));
        assert_eq!(
            apply_event(Some("p"), &mut failed, dpi()),
            EventEffect::Nothing
        );
        assert_eq!(
            apply_event(None, &mut none, Event::ShuttingDown),
            EventEffect::Nothing
        );
        assert_eq!(
            apply_event(
                None,
                &mut none,
                Event::Error {
                    message: "x".into()
                }
            ),
            EventEffect::Note("agent: x".into())
        );
    }

    #[test]
    fn app_profile_switch_rereads_the_selected_device() {
        let ev = || Event::AppProfile {
            exe: r"C:\Windows
otepad.exe"
                .into(),
            profile: Some("profiles/notepad.toml".into()),
            lines: vec![],
            error: None,
        };
        let mut snap = Some(Ok(snapshot()));
        assert_eq!(apply_event(Some("p"), &mut snap, ev()), EventEffect::Reread);
        let mut none = None;
        assert_eq!(apply_event(Some("p"), &mut none, ev()), EventEffect::Reread);
        assert_eq!(apply_event(None, &mut none, ev()), EventEffect::Nothing);
    }
}
