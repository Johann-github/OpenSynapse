//! HID worker thread: owns the transport and the open session, so device I/O
//! (25 ms pacing per report) never blocks the UI.
//!
//! The thread sleeps in `recv()` until the UI sends a job; after each reply it
//! asks egui for one repaint. No timers, no polling.

use std::sync::mpsc::{Receiver, Sender, channel};

use opensynapse_device_registry::{Device, Protocol, RAZER_VID, Registry};
use opensynapse_hid::{DEFAULT_INTERFACE, DeviceSession, HidEntry, Timing, Transport};

use crate::ops::{self, Setting, Snapshot, WriteFailure, WriteOutcome};

pub enum Job {
    /// Enumerate devices and check for conflicting Razer software.
    Scan,
    /// Read the whole panel of one device.
    Read(HidEntry),
    /// Guarded write (see [`ops::guarded_write`]).
    Write {
        entry: HidEntry,
        setting: Setting,
        confirmed: bool,
    },
}

pub enum Reply {
    Devices {
        list: Result<Vec<DeviceRow>, String>,
        conflict: Option<&'static str>,
    },
    Snapshot {
        path: String,
        result: Result<Snapshot, String>,
    },
    Written {
        path: String,
        result: Result<WriteOutcome, WriteFailure>,
        conflict: Option<&'static str>,
    },
}

/// One control collection plus what the registry knows about it.
#[derive(Clone)]
pub struct DeviceRow {
    pub entry: HidEntry,
    pub device: Option<&'static Device>,
    pub protocol: Protocol,
}

impl DeviceRow {
    fn new(entry: HidEntry) -> Self {
        let m = Registry::embedded().lookup(RAZER_VID, entry.pid);
        Self {
            device: m.map(|m| m.device()),
            protocol: m.map_or(Protocol::Unknown, |m| m.protocol()),
            entry,
        }
    }

    pub fn name(&self) -> &str {
        self.device
            .map_or("unknown Razer device", |d| d.name.as_str())
    }

    /// Only protocol 2.5 is implemented.
    pub fn supported(&self) -> bool {
        self.protocol == Protocol::V25
    }
}

/// Start the worker. Dropping the returned `Sender` ends the thread.
pub fn spawn(ctx: eframe::egui::Context) -> (Sender<Job>, Receiver<Reply>) {
    let (job_tx, job_rx) = channel::<Job>();
    let (reply_tx, reply_rx) = channel::<Reply>();
    std::thread::Builder::new()
        .name("opensynapse-hid".into())
        .spawn(move || {
            let mut w = Worker::default();
            while let Ok(job) = job_rx.recv() {
                if reply_tx.send(w.handle(job)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        })
        .expect("spawn HID worker thread");
    (job_tx, reply_rx)
}

#[derive(Default)]
struct Worker {
    transport: Option<Transport>,
    /// Path of the open collection and its session.
    session: Option<(String, DeviceSession)>,
}

impl Worker {
    fn handle(&mut self, job: Job) -> Reply {
        match job {
            Job::Scan => {
                self.session = None;
                self.transport = None;
                let list = Transport::new()
                    .map(|t| {
                        let rows = t
                            .control_entries(DEFAULT_INTERFACE)
                            .into_iter()
                            .map(DeviceRow::new)
                            .collect();
                        self.transport = Some(t);
                        rows
                    })
                    .map_err(|e| format!("cannot enumerate HID devices: {e}"));
                Reply::Devices {
                    list,
                    conflict: opensynapse_hid::conflicting_process(),
                }
            }
            Job::Read(entry) => {
                let result = self
                    .session(&entry)
                    .and_then(|(_, s)| ops::read_snapshot(s));
                if result.is_err() {
                    // Reopen next time; the device may have been replugged.
                    self.session = None;
                }
                Reply::Snapshot {
                    path: entry.path,
                    result,
                }
            }
            Job::Write {
                entry,
                setting,
                confirmed,
            } => {
                let conflict = opensynapse_hid::conflicting_process();
                let result = match self.session(&entry) {
                    Ok((row, s)) => {
                        if row.supported() {
                            ops::guarded_write(s, setting, row.device, conflict, confirmed)
                        } else {
                            Err(WriteFailure {
                                old: None,
                                message: format!(
                                    "protocol {} is not implemented; writes are disabled",
                                    row.protocol
                                ),
                            })
                        }
                    }
                    Err(message) => Err(WriteFailure { old: None, message }),
                };
                Reply::Written {
                    path: entry.path,
                    result,
                    conflict,
                }
            }
        }
    }

    /// Open (or reuse) the session for `entry`.
    fn session(&mut self, entry: &HidEntry) -> Result<(DeviceRow, &mut DeviceSession), String> {
        let row = DeviceRow::new(entry.clone());
        if !row.supported() {
            return Err(format!(
                "{} speaks protocol {}; only 2.5 is implemented",
                row.name(),
                row.protocol
            ));
        }
        if self.session.as_ref().is_none_or(|(p, _)| *p != entry.path) {
            self.session = None;
            if self.transport.is_none() {
                self.transport =
                    Some(Transport::new().map_err(|e| format!("cannot enumerate: {e}"))?);
            }
            let t = self.transport.as_ref().expect("set above");
            let s = t
                .open(entry, Timing::default())
                .map_err(|e| format!("cannot open device: {e}"))?;
            self.session = Some((entry.path.clone(), s));
        }
        let (_, s) = self.session.as_mut().expect("set above");
        Ok((row, s))
    }
}
