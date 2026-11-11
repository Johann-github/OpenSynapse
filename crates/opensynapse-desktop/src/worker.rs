//! Worker thread: owns the backend, so device I/O (25 ms pacing per report, or an
//! IPC round trip) never blocks the UI.
//!
//! Two backends ([`crate::backend`]): the background agent over IPC when it runs,
//! else in-process HID (the transport and the open session live here). With the
//! agent, a second thread holds a `Subscribe` connection and forwards its events.
//!
//! Both threads sleep in a blocking `recv()`/read until something happens; after
//! each reply or event they ask egui for one repaint. No timers, no polling.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use eframe::egui::Context;
use opensynapse_device_registry::{Device, Protocol, RAZER_VID, Registry};
use opensynapse_hid::{DEFAULT_INTERFACE, DeviceSession, HidEntry, Timing, Transport};
use opensynapse_ipc::Client;
use opensynapse_ipc::msg::{DeviceRef, ErrorKind, Event, IpcError, Reply as IpcReply, Request};

use opensynapse_sys::agent_exit;

use crate::agent_ctl::{self, StartFailure};
use crate::backend::{self, BackendState};
use crate::ops::{self, Setting, Snapshot, WriteFailure, WriteOpts, WriteOutcome};
use crate::remote;

pub enum Job {
    /// Try the agent; fall back to direct HID.
    Connect,
    /// Start the agent detached, wait briefly until it answers, then use it
    /// (as `Connect`). Bounded wait on this thread, never on the UI thread.
    StartAgent,
    /// The event subscription ended: drop the agent, use direct HID.
    UseLocal(Option<String>),
    /// List devices and check for conflicting Razer software. `reselect`: read
    /// the selected device again even if it is still there.
    Scan { reselect: bool },
    /// Read the whole panel of one device (by control-collection path).
    Read(String),
    /// Guarded write (see [`ops::guarded_write`]).
    Write {
        path: String,
        setting: Setting,
        opts: WriteOpts,
    },
}

pub enum Reply {
    /// Answer to `Connect` / `StartAgent` / `UseLocal`.
    Backend(BackendState),
    /// Not an answer: the agent connection broke during a job.
    Switched(BackendState),
    Devices {
        list: Result<Vec<DeviceRow>, String>,
        conflict: Option<String>,
        reselect: bool,
    },
    Snapshot {
        path: String,
        result: Result<Snapshot, String>,
    },
    Written {
        path: String,
        result: Result<WriteOutcome, WriteFailure>,
        conflict: Option<String>,
    },
}

impl Reply {
    /// Answers one job (the UI counts pending jobs).
    pub fn is_answer(&self) -> bool {
        !matches!(self, Self::Switched(_))
    }
}

/// From the event thread.
pub enum SubMsg {
    Event(Event),
    /// The subscription ended; the error, if it was not a clean close.
    Ended(Option<String>),
}

/// One control collection plus what the registry knows about it.
#[derive(Clone)]
pub struct DeviceRow {
    pub path: String,
    pub pid: u16,
    pub device: Option<&'static Device>,
    pub protocol: Protocol,
}

impl DeviceRow {
    pub fn new(path: String, pid: u16) -> Self {
        let m = Registry::embedded().lookup(RAZER_VID, pid);
        Self {
            device: m.map(|m| m.device()),
            protocol: m.map_or(Protocol::Unknown, |m| m.protocol()),
            path,
            pid,
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
pub fn spawn(ctx: Context) -> (Sender<Job>, Receiver<Reply>, Receiver<SubMsg>) {
    let (job_tx, job_rx) = channel::<Job>();
    let (reply_tx, reply_rx) = channel::<Reply>();
    let (event_tx, event_rx) = channel::<SubMsg>();
    std::thread::Builder::new()
        .name("opensynapse-worker".into())
        .spawn(move || {
            let mut w = Worker {
                ctx: ctx.clone(),
                events: event_tx,
                backend: Backend::Local(Local::default()),
            };
            while let Ok(job) = job_rx.recv() {
                let (reply, switched) = w.handle(job);
                let sent = switched
                    .into_iter()
                    .map(Reply::Switched)
                    .chain([reply])
                    .all(|r| reply_tx.send(r).is_ok());
                if !sent {
                    break;
                }
                ctx.request_repaint();
            }
        })
        .expect("spawn worker thread");
    (job_tx, reply_rx, event_rx)
}

/// Forward agent events until the agent closes the connection.
fn spawn_subscription(tx: Sender<SubMsg>, ctx: Context) {
    let run = move || {
        let end = match Client::subscribe() {
            Ok(sub) => {
                let mut end = None;
                for ev in sub {
                    match ev {
                        Ok(e) => {
                            if tx.send(SubMsg::Event(e)).is_err() {
                                return; // the window is gone
                            }
                            ctx.request_repaint();
                        }
                        Err(e) => end = Some(e.to_string()),
                    }
                }
                end
            }
            Err(e) => Some(e.to_string()),
        };
        let _ = tx.send(SubMsg::Ended(end));
        ctx.request_repaint();
    };
    if let Err(e) = std::thread::Builder::new()
        .name("opensynapse-events".into())
        .spawn(run)
    {
        // Without events the agent still works; the user can rescan by hand.
        eprintln!("cannot start the event thread: {e}");
    }
}

enum Backend {
    Local(Local),
    Agent(Agent),
}

struct Worker {
    ctx: Context,
    events: Sender<SubMsg>,
    backend: Backend,
}

impl Worker {
    /// The job's answer, plus the new backend if the agent connection broke.
    fn handle(&mut self, job: Job) -> (Reply, Option<BackendState>) {
        match job {
            Job::Connect => (Reply::Backend(self.connect()), None),
            Job::StartAgent => {
                let state = self.start_agent().unwrap_or_else(|f| BackendState::Local {
                    note: Some(agent_ctl::start_failure_note(&f)),
                });
                (Reply::Backend(state), None)
            }
            Job::UseLocal(reason) => {
                self.backend = Backend::Local(Local::default());
                (Reply::Backend(backend::lost(reason.as_deref())), None)
            }
            job => {
                let result = match &mut self.backend {
                    Backend::Local(l) => Ok(l.handle(job)),
                    Backend::Agent(a) => a.handle(job),
                };
                match result {
                    Ok(reply) => (reply, None),
                    Err(broken) => {
                        let (reply, e) = *broken;
                        self.backend = Backend::Local(Local::default());
                        (reply, Some(backend::lost(Some(&e.to_string()))))
                    }
                }
            }
        }
    }
}

impl Worker {
    /// Try the agent; keep or fall back to direct HID.
    fn connect(&mut self) -> BackendState {
        let (client, state) = backend::select(Client::connect(), Client::agent_pid);
        if let Some(client) = client {
            self.backend = Backend::Agent(Agent {
                client,
                conflict: None,
            });
            spawn_subscription(self.events.clone(), self.ctx.clone());
        } else if matches!(self.backend, Backend::Agent(_)) {
            self.backend = Backend::Local(Local::default());
        }
        state
    }

    /// Spawn the agent (installed, else next to the GUI) and connect once its pipe answers. Waits
    /// at most [`agent_ctl::START_WAIT`]; a slow agent keeps running and can be
    /// reached later with "Reconnect to agent".
    fn start_agent(&mut self) -> Result<BackendState, StartFailure> {
        let state = self.connect();
        if state.is_agent() {
            return Ok(state); // started elsewhere in the meantime
        }
        let mut child = opensynapse_sys::spawn_agent_detached()
            .map_err(|e| StartFailure::Spawn(e.to_string()))?;
        let deadline = Instant::now() + agent_ctl::START_WAIT;
        loop {
            let state = self.connect();
            if state.is_agent() {
                return Ok(state);
            }
            if let Ok(Some(status)) = child.try_wait() {
                // Another agent won the race for the pipe: use that one.
                if status.code() == Some(agent_exit::ALREADY_RUNNING) {
                    let state = self.connect();
                    if state.is_agent() {
                        return Ok(state);
                    }
                }
                return Err(StartFailure::Exited(status.code()));
            }
            if Instant::now() >= deadline {
                return Err(StartFailure::NoAnswer { pid: child.id() });
            }
            std::thread::sleep(agent_ctl::START_STEP);
        }
    }
}

/// In-process HID.
#[derive(Default)]
struct Local {
    transport: Option<Transport>,
    /// Control collections from the last scan, by path.
    entries: HashMap<String, HidEntry>,
    /// Path of the open collection and its session.
    session: Option<(String, DeviceSession)>,
}

impl Local {
    fn handle(&mut self, job: Job) -> Reply {
        let conflict = || opensynapse_hid::conflicting_process().map(str::to_owned);
        match job {
            Job::Connect | Job::StartAgent | Job::UseLocal(_) => {
                unreachable!("handled by the worker")
            }
            Job::Scan { reselect } => {
                self.session = None;
                self.transport = None;
                self.entries.clear();
                let list = Transport::new()
                    .map(|t| {
                        let entries = t.control_entries(DEFAULT_INTERFACE);
                        self.transport = Some(t);
                        entries
                            .into_iter()
                            .map(|e| {
                                let row = DeviceRow::new(e.path.clone(), e.pid);
                                self.entries.insert(e.path.clone(), e);
                                row
                            })
                            .collect()
                    })
                    .map_err(|e| format!("cannot enumerate HID devices: {e}"));
                Reply::Devices {
                    list,
                    conflict: conflict(),
                    reselect,
                }
            }
            Job::Read(path) => {
                let result = self.session(&path).and_then(|(_, s)| ops::read_snapshot(s));
                if result.is_err() {
                    // Reopen next time; the device may have been replugged.
                    self.session = None;
                }
                Reply::Snapshot { path, result }
            }
            Job::Write {
                path,
                setting,
                opts,
            } => {
                let conflict = conflict();
                let result = match self.session(&path) {
                    Ok((row, s)) => {
                        ops::guarded_write(s, setting, row.device, conflict.as_deref(), opts)
                    }
                    Err(message) => Err(WriteFailure { old: None, message }),
                };
                Reply::Written {
                    path,
                    result,
                    conflict,
                }
            }
        }
    }

    /// Open (or reuse) the session for the collection at `path`. Refuses devices
    /// whose protocol is not implemented.
    fn session(&mut self, path: &str) -> Result<(DeviceRow, &mut DeviceSession), String> {
        let entry = self
            .entries
            .get(path)
            .ok_or_else(|| "device is gone; rescan".to_owned())?;
        let row = DeviceRow::new(entry.path.clone(), entry.pid);
        if !row.supported() {
            return Err(format!(
                "{} speaks protocol {}; only 2.5 is implemented",
                row.name(),
                row.protocol
            ));
        }
        if self.session.as_ref().is_none_or(|(p, _)| p != path) {
            self.session = None;
            if self.transport.is_none() {
                self.transport =
                    Some(Transport::new().map_err(|e| format!("cannot enumerate: {e}"))?);
            }
            let t = self.transport.as_ref().expect("set above");
            let s = t
                .open(entry, Timing::default())
                .map_err(|e| format!("cannot open device: {e}"))?;
            self.session = Some((path.to_owned(), s));
        }
        let (_, s) = self.session.as_mut().expect("set above");
        Ok((row, s))
    }
}

/// The background agent over a `Control` connection. It keeps the same guards
/// (validate, conflict gate, read old -> write -> read back).
struct Agent {
    client: Client,
    /// Razer software reported by the last `ListDevices`.
    conflict: Option<String>,
}

// Boxed: a broken connection is rare, the reply is large.
type Broken = Box<(Reply, io::Error)>;

impl Agent {
    /// `Err` when the connection broke; the reply then carries that error.
    fn handle(&mut self, job: Job) -> Result<Reply, Broken> {
        let lost = |e: &io::Error| format!("agent connection lost: {e}");
        match job {
            Job::Connect | Job::StartAgent | Job::UseLocal(_) => {
                unreachable!("handled by the worker")
            }
            Job::Scan { reselect } => match self.client.call(Request::ListDevices) {
                Ok(r) => {
                    let list = match r {
                        Ok(IpcReply::Devices { list, conflict }) => {
                            self.conflict = conflict;
                            Ok(list
                                .into_iter()
                                .map(|d| DeviceRow::new(d.path, d.pid))
                                .collect())
                        }
                        Ok(other) => Err(format!("agent sent an unexpected reply: {other:?}")),
                        Err(IpcError { message, .. }) => Err(message),
                    };
                    Ok(Reply::Devices {
                        list,
                        conflict: self.conflict.clone(),
                        reselect,
                    })
                }
                Err(e) => Err(Box::new((
                    Reply::Devices {
                        list: Err(lost(&e)),
                        conflict: None,
                        reselect,
                    },
                    e,
                ))),
            },
            Job::Read(path) => {
                let req = Request::ReadSnapshot {
                    device: DeviceRef::Path(path.clone()),
                };
                match self.client.call(req) {
                    Ok(r) => Ok(Reply::Snapshot {
                        path,
                        result: remote::snapshot_reply(r),
                    }),
                    Err(e) => Err(Box::new((
                        Reply::Snapshot {
                            path,
                            result: Err(lost(&e)),
                        },
                        e,
                    ))),
                }
            }
            Job::Write {
                path,
                setting,
                opts,
            } => {
                let req = Request::Write {
                    device: DeviceRef::Path(path.clone()),
                    setting: remote::setting_to_wire(setting),
                    opts: remote::opts_to_wire(opts),
                };
                match self.client.call(req) {
                    Ok(r) => {
                        if matches!(&r, Err(e) if e.kind == ErrorKind::Conflict) {
                            // Razer software started since the last list: learn
                            // its name so the UI offers the confirmation box.
                            if let Ok(Ok(IpcReply::Devices { conflict, .. })) =
                                self.client.call(Request::ListDevices)
                            {
                                self.conflict = conflict;
                            }
                        }
                        Ok(Reply::Written {
                            path,
                            result: remote::written_reply(r),
                            conflict: self.conflict.clone(),
                        })
                    }
                    Err(e) => Err(Box::new((
                        Reply::Written {
                            path,
                            // Unknown whether the agent wrote; the read after
                            // the switch shows the device's value.
                            result: Err(WriteFailure {
                                old: None,
                                message: lost(&e),
                            }),
                            conflict: None,
                        },
                        e,
                    ))),
                }
            }
        }
    }
}
