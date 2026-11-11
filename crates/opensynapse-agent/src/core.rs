//! The core actor: the only owner of HID sessions. Runs on the main thread and
//! sleeps in `recv()` until a client request, a hotplug rescan, or a device event
//! arrives. No timers, no polling.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use opensynapse_core::Profile;
use opensynapse_device as dev;
use opensynapse_device::profile_apply::{self as pa, ApplyError, ApplyOptions, ApplyReport};
use opensynapse_hid::{HidEntry, Session};
use opensynapse_ipc::msg::{DeviceRef, ErrorKind, Event, IpcError, Reply, Request, WireWriteOpts};
use opensynapse_protocol::dpi::Storage;

use crate::convert;
use crate::events::{Detach, EventSink, EventSource};
use crate::hardware::{Hardware, is_control};

/// Quiet time after the first hotplug notification of a burst (one device brings
/// several interfaces); later notifications of the burst are merged.
pub const SETTLE: Duration = Duration::from_millis(150);

/// Messages to the core.
pub enum CoreMsg {
    /// A control request; the answer goes back on `reply`.
    Request {
        req: Request,
        reply: Sender<Result<Reply, IpcError>>,
    },
    /// A `Subscribe` connection wants events.
    Subscribe(Sender<Event>),
    /// Hotplug: re-enumerate (coalesced via the shared pending flag).
    Rescan,
    /// A hardware event from an [`EventSource`].
    DeviceEvent(Event),
    /// The foreground application changed (full executable path); per-app profiles.
    Foreground { exe: String },
}

/// `outer: cause: cause…`, so a read error keeps its HID reason.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

/// `Device`, or `NoResponse` when the device did not answer at all (asleep).
/// `message` is the full text; the class comes from `opensynapse-hid`.
fn device_kind(message: &str) -> ErrorKind {
    if opensynapse_hid::is_no_response_text(message) {
        ErrorKind::NoResponse
    } else {
        ErrorKind::Device
    }
}

fn err(kind: ErrorKind, message: impl Into<String>) -> IpcError {
    IpcError {
        kind,
        message: message.into(),
    }
}

pub struct Core<H: Hardware> {
    hw: H,
    events: Box<dyn EventSource>,
    tx: Sender<CoreMsg>,
    pending: Arc<AtomicBool>,
    settle: Duration,
    /// Control collections by path, from the last rescan.
    devices: BTreeMap<String, HidEntry>,
    sessions: HashMap<String, Session<H::Io>>,
    detachers: HashMap<String, Box<dyn Detach>>,
    subscribers: Vec<Sender<Event>>,
    /// No hotplug watch: re-enumerate on every `ListDevices` instead.
    rescan_on_list: bool,
    /// Per-app profiles (`core/apps.rs`).
    apps: apps::AppState,
}

impl<H: Hardware> Core<H> {
    pub fn new(
        hw: H,
        events: Box<dyn EventSource>,
        tx: Sender<CoreMsg>,
        pending: Arc<AtomicBool>,
        settle: Duration,
    ) -> Self {
        Self {
            hw,
            events,
            tx,
            pending,
            settle,
            devices: BTreeMap::new(),
            sessions: HashMap::new(),
            detachers: HashMap::new(),
            subscribers: Vec::new(),
            rescan_on_list: false,
            apps: apps::AppState::default(),
        }
    }

    pub fn set_rescan_on_list(&mut self, on: bool) {
        self.rescan_on_list = on;
    }

    /// Serve messages until a `Shutdown` request was answered.
    pub fn run(&mut self, rx: &Receiver<CoreMsg>) {
        while let Ok(msg) = rx.recv() {
            if self.dispatch(msg) {
                break;
            }
        }
        self.stop();
    }

    /// Handle one message; `true` after `Shutdown`.
    pub fn dispatch(&mut self, msg: CoreMsg) -> bool {
        match msg {
            CoreMsg::Request { req, reply } => {
                let shutdown = matches!(req, Request::Shutdown);
                let _ = reply.send(self.handle(req));
                shutdown
            }
            CoreMsg::Subscribe(s) => {
                self.subscribers.push(s);
                false
            }
            CoreMsg::Rescan => {
                // One sleep per burst; notifications during it only set the flag.
                std::thread::sleep(self.settle);
                self.pending.store(false, Ordering::SeqCst);
                self.rescan();
                false
            }
            CoreMsg::DeviceEvent(ev) => {
                self.broadcast(ev);
                false
            }
            CoreMsg::Foreground { exe } => {
                self.on_foreground(exe);
                false
            }
        }
    }

    /// Tell subscribers, detach event sources, close sessions.
    fn stop(&mut self) {
        self.broadcast(Event::ShuttingDown);
        for (_, d) in self.detachers.drain() {
            d.detach();
        }
        self.sessions.clear();
        self.subscribers.clear();
    }

    #[cfg(test)]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    /// Send to every subscriber; drop the ones whose connection is gone.
    pub fn broadcast(&mut self, ev: Event) {
        self.subscribers.retain(|s| s.send(ev.clone()).is_ok());
    }

    /// Re-enumerate and report what arrived and what went away.
    pub fn rescan(&mut self) {
        let all = match self.hw.enumerate() {
            Ok(all) => all,
            Err(message) => {
                tracing::warn!(%message, "rescan failed");
                self.broadcast(Event::Error { message });
                return;
            }
        };
        let now: BTreeMap<String, HidEntry> = all
            .iter()
            .filter(|e| is_control(e))
            .map(|e| (e.path.clone(), e.clone()))
            .collect();

        let removed: Vec<HidEntry> = self
            .devices
            .values()
            .filter(|e| !now.contains_key(&e.path))
            .cloned()
            .collect();
        for e in removed {
            tracing::info!(path = %e.path, pid = e.pid, "device removed");
            self.sessions.remove(&e.path);
            if let Some(d) = self.detachers.remove(&e.path) {
                d.detach();
            }
            self.devices.remove(&e.path);
            self.broadcast(Event::DeviceRemoved {
                path: e.path,
                pid: e.pid,
            });
        }

        for (path, e) in now {
            if self.devices.contains_key(&path) {
                continue;
            }
            tracing::info!(%path, pid = e.pid, "device arrived");
            let siblings: Vec<HidEntry> = all
                .iter()
                .filter(|s| s.pid == e.pid && s.path != path)
                .cloned()
                .collect();
            let sink = EventSink::new(self.tx.clone(), path.clone());
            if let Some(d) = self.events.attach(&e, &siblings, sink) {
                self.detachers.insert(path.clone(), d);
            }
            self.broadcast(Event::DeviceArrived(convert::device_info(&e)));
            self.devices.insert(path, e);
        }
    }

    /// Answer one control request.
    pub fn handle(&mut self, req: Request) -> Result<Reply, IpcError> {
        match req {
            Request::Ping => Ok(Reply::Pong {
                agent_version: env!("CARGO_PKG_VERSION").into(),
            }),
            Request::ListDevices => {
                if self.rescan_on_list {
                    self.rescan();
                }
                Ok(Reply::Devices {
                    list: self.devices.values().map(convert::device_info).collect(),
                    conflict: self.hw.conflict(),
                })
            }
            Request::ReadSnapshot { device } => {
                let path = self.resolve(&device)?;
                let s = self.session(&path)?;
                match dev::read_snapshot(s) {
                    Ok(snap) => Ok(Reply::Snapshot(convert::snapshot_to_wire(snap))),
                    Err(message) => {
                        // Reopen next time; the device may have been replugged.
                        self.sessions.remove(&path);
                        Err(err(device_kind(&message), message))
                    }
                }
            }
            Request::Write {
                device,
                setting,
                opts,
            } => {
                let path = self.resolve(&device)?;
                // Checked at write time, never on a timer.
                let conflict = self.hw.conflict();
                if let Some(name) = &conflict
                    && !opts.confirmed
                {
                    return Err(err(
                        ErrorKind::Conflict,
                        format!(
                            "{name} is running and may talk to the device at the same time; \
                             close it or confirm the write"
                        ),
                    ));
                }
                let device = convert::registry_device(self.devices[&path].pid);
                // Refuses devices whose protocol is not implemented.
                let s = self.session(&path)?;
                let result = dev::guarded_write(
                    s,
                    convert::setting_from_wire(setting),
                    device,
                    conflict.as_deref(),
                    convert::opts_from_wire(opts),
                );
                if result.is_err() {
                    self.sessions.remove(&path);
                }
                Ok(Reply::Written(
                    result
                        .map(convert::outcome_to_wire)
                        .map_err(convert::failure_to_wire),
                ))
            }
            Request::ApplyProfile {
                device,
                toml,
                dry_run,
                all,
                opts,
            } => self.apply_profile(&device, &toml, dry_run, all, opts),
            Request::Shutdown => Ok(Reply::ShuttingDown),
            Request::ReloadAppProfiles => Ok(self.reload_app_profiles()),
            Request::AppProfiles => Ok(Reply::AppProfiles(self.app_profiles_info())),
        }
    }

    /// `ApplyProfile` through the shared engine (`opensynapse_device::profile_apply`,
    /// same as `opensynapse profile apply`). Persistent DPI storage. `confirmed`
    /// plays the CLI's `--force`: it passes the Razer-software gate and the PID,
    /// offered-rate and snapshot checks. The restore profile goes back as TOML; the
    /// agent never writes files.
    fn apply_profile(
        &mut self,
        device: &DeviceRef,
        toml: &str,
        dry_run: bool,
        all: bool,
        opts: WireWriteOpts,
    ) -> Result<Reply, IpcError> {
        let path = self.resolve(device)?;
        let profile =
            Profile::from_toml(toml).map_err(|e| err(ErrorKind::BadRequest, e.to_string()))?;
        let options = ApplyOptions {
            storage: Storage::Persistent,
            all,
            force: opts.confirmed,
        };
        let pid = self.devices[&path].pid;
        let name = convert::registry_device(pid).map_or("unknown device", |d| d.name.as_str());
        let report = self.run_apply(&path, profile, options, dry_run)?;
        let restore_toml = match &report.restore {
            Some(p) => Some(
                pa::render(p, &pa::header(name, pid, "values before applying to"))
                    .map_err(|e| err(ErrorKind::Device, e.to_string()))?,
            ),
            None => None,
        };
        Ok(Reply::ProfileApplied {
            lines: report.lines,
            restore_toml,
            error: report.failed,
        })
    }

    /// Run the shared engine on the device at `path` (one of `self.devices`).
    /// Razer software is checked here, at write time. Errors before the first write
    /// are `Err`; a failed write is a report with `failed` set. The session is
    /// dropped after a device error so the next call reopens it.
    fn run_apply(
        &mut self,
        path: &str,
        profile: Profile,
        options: ApplyOptions,
        dry_run: bool,
    ) -> Result<ApplyReport, IpcError> {
        let conflict = self.hw.conflict();
        let pid = self.devices[path].pid;
        let registry = convert::registry_device(pid);
        let s = self.session(path)?;
        match pa::apply(
            s,
            pid,
            registry,
            conflict.as_deref(),
            profile,
            options,
            dry_run,
        ) {
            Ok(r) => {
                if r.failed.is_some() {
                    self.sessions.remove(path);
                }
                Ok(r)
            }
            Err(e) => {
                let kind = match e {
                    ApplyError::Conflict(_) => ErrorKind::Conflict,
                    ApplyError::Read { .. } | ApplyError::Write { .. } => {
                        // Reopen next time; the device may have been replugged.
                        self.sessions.remove(path);
                        device_kind(&error_chain(&e))
                    }
                    _ => ErrorKind::BadRequest,
                };
                Err(err(kind, error_chain(&e)))
            }
        }
    }

    /// The control-collection path a request targets.
    fn resolve(&self, r: &DeviceRef) -> Result<String, IpcError> {
        match r {
            DeviceRef::Path(p) => self
                .devices
                .get(p)
                .map(|e| e.path.clone())
                .ok_or_else(|| err(ErrorKind::NoSuchDevice, format!("no device at {p}"))),
            DeviceRef::Pid(pid) => {
                let mut hits = self.devices.values().filter(|e| e.pid == *pid);
                match (hits.next(), hits.next()) {
                    (None, _) => Err(err(
                        ErrorKind::NoSuchDevice,
                        format!("no device with PID {pid:#06x}"),
                    )),
                    (Some(e), None) => Ok(e.path.clone()),
                    (Some(_), Some(_)) => Err(err(
                        ErrorKind::Ambiguous,
                        format!("several devices with PID {pid:#06x}; pick one by path"),
                    )),
                }
            }
        }
    }

    /// Open (or reuse) the session of a supported device.
    fn session(&mut self, path: &str) -> Result<&mut Session<H::Io>, IpcError> {
        let entry = self
            .devices
            .get(path)
            .ok_or_else(|| err(ErrorKind::NoSuchDevice, format!("no device at {path}")))?;
        let (name, protocol) = convert::registry_facts(entry.pid);
        if !convert::is_supported(protocol) {
            return Err(err(
                ErrorKind::Unsupported,
                format!(
                    "{} speaks protocol {protocol}; only 2.5 is implemented",
                    name.unwrap_or("unknown Razer device")
                ),
            ));
        }
        if !self.sessions.contains_key(path) {
            let s = self.hw.open(entry).map_err(|m| err(ErrorKind::Device, m))?;
            self.sessions.insert(path.to_owned(), s);
        }
        Ok(self.sessions.get_mut(path).expect("inserted above"))
    }

    #[cfg(test)]
    pub fn hw(&mut self) -> &mut H {
        &mut self.hw
    }

    #[cfg(test)]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

mod apps;

#[cfg(test)]
mod tests;
