//! HID transport and device sessions. See README.md.
//!
//! Transport facts: `vault/Protocol/Windows Transport.md`. Retry/busy rules:
//! `vault/Protocol/Report Format 2.5.md`. This crate never builds report bytes
//! itself; it moves [`opensynapse_protocol::Report`]s.
//!
//! Backends: native SetupAPI/hid.dll on Windows (`windows.rs`, see
//! `vault/Decisions/ADR-0003 Own Windows HID backend.md`), hidapi elsewhere.

use std::time::Duration;

use opensynapse_device_registry::RAZER_VID;
use opensynapse_protocol::{
    Disposition, FEATURE_REPORT_LEN, ProtocolError, Report, Request, Status, TransactionCounter,
};

pub mod hotplug;
pub mod input;
#[cfg(target_os = "linux")]
mod linux_hotplug;
mod uevent;
#[cfg(windows)]
mod windows_hotplug;
#[cfg(windows)]
mod windows_input;

#[cfg(not(windows))]
mod hidapi_backend;
#[cfg(not(windows))]
use hidapi_backend as backend;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;

/// Usage page / usage of the collection that carries the 91-byte feature report.
pub const CONTROL_USAGE_PAGE: u16 = 0x0001;
pub const CONTROL_USAGE: u16 = 0x0002;
/// Interface Synapse claims for the Viper V3 HyperSpeed (and the 2.5 default).
pub const DEFAULT_INTERFACE: i32 = 0;

/// Process image names of Razer software that drives the same devices. Both apps
/// can hold the control collection at once and interleave reports.
const CONFLICTING: &[&str] = &["RazerAppEngine.exe"];

#[derive(Debug, thiserror::Error)]
pub enum HidError {
    #[error("HID I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("device answered {0:?}")]
    Status(Status),
    #[error("no answer after {attempts} attempts (last problem: {last})")]
    Exhausted { attempts: u32, last: String },
    /// Every send was used up and the last reply carried status Timeout (0x04).
    /// A wireless mouse behind its dongle answers like this while it sleeps
    /// (observed 2026-11-10, `vault/RE-Log/2026-11-10 Agent slice 1 test.md`);
    /// out of range or switched off look the same. Never retried further here.
    #[error("{msg} (status Timeout after {attempts} attempts)", msg = NO_RESPONSE)]
    NoResponse { attempts: u32 },
}

/// Stable start of the [`HidError::NoResponse`] text. Errors cross string
/// boundaries (snapshot fields, write failures, IPC messages); this phrase lets
/// those layers still recognise the class via [`is_no_response_text`]. Owner of
/// the "device asleep" classification: this crate. See
/// `vault/Decisions/ADR-0008 User settings and wake hint.md`.
pub const NO_RESPONSE: &str = "no response from the device";

impl HidError {
    /// The device did not answer at all (status Timeout until the retries ran
    /// out): typically a sleeping wireless mouse.
    pub fn is_no_response(&self) -> bool {
        matches!(self, Self::NoResponse { .. })
    }
}

/// True when an error message (possibly wrapped: `cannot read DPI: …`) carries
/// the [`HidError::NoResponse`] text.
pub fn is_no_response_text(message: &str) -> bool {
    message.contains(NO_RESPONSE)
}

/// One HID collection of a Razer device, as enumerated.
#[derive(Clone, Debug)]
pub struct HidEntry {
    pub vid: u16,
    pub pid: u16,
    /// USB interface number, -1 if unknown / not composite.
    pub interface: i32,
    pub usage_page: u16,
    pub usage: u16,
    /// Feature report length incl. report ID, if the backend can tell.
    pub feature_len: Option<u16>,
    /// Input report length incl. report ID, if the backend can tell (0 = none).
    pub input_len: Option<u16>,
    /// Output report length incl. report ID, if the backend can tell (0 = none).
    pub output_len: Option<u16>,
    pub path: String,
}

/// One HID collection plus whether it can be opened for reading input reports.
/// See [`collections`].
#[derive(Clone, Debug)]
pub struct Collection {
    pub entry: HidEntry,
    /// A shared `GENERIC_READ` open succeeded (Windows). The OS holds mouse and
    /// keyboard collections exclusively, so those report `false`.
    pub readable: bool,
}

/// Every HID collection of vendor `vid` (all vendors with `None`), sorted by
/// PID and path, each probed for read access. Diagnostic: the probe opens and
/// closes every matching collection once; nothing is read or written.
pub fn collections(vid: Option<u16>) -> Result<Vec<Collection>, HidError> {
    let mut entries = backend::enumerate_any(vid)?;
    entries.sort_by(|a, b| (a.vid, a.pid, &a.path).cmp(&(b.vid, b.pid, &b.path)));
    entries.dedup_by(|a, b| a.path == b.path);
    Ok(entries
        .into_iter()
        .map(|entry| Collection {
            readable: backend::can_read(&entry.path),
            entry,
        })
        .collect())
}

impl HidEntry {
    /// True for the collection that carries the protocol 2.5 feature report.
    pub fn is_control(&self, interface: i32) -> bool {
        self.interface == interface
            && self.usage_page == CONTROL_USAGE_PAGE
            && self.usage == CONTROL_USAGE
            && self
                .feature_len
                .is_none_or(|n| usize::from(n) == FEATURE_REPORT_LEN)
    }
}

/// Enumerated Razer HID collections.
pub struct Transport {
    entries: Vec<HidEntry>,
}

impl Transport {
    /// Enumerate Razer (VID 0x1532) collections.
    pub fn new() -> Result<Self, HidError> {
        let mut entries = backend::enumerate_any(Some(RAZER_VID))?;
        entries.sort_by(|a, b| (a.pid, &a.path).cmp(&(b.pid, &b.path)));
        entries.dedup_by(|a, b| a.path == b.path);
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[HidEntry] {
        &self.entries
    }

    /// Control collections (usage page 1 / usage 2 / 91-byte feature on `interface`).
    pub fn control_entries(&self, interface: i32) -> Vec<HidEntry> {
        self.entries
            .iter()
            .filter(|e| e.is_control(interface))
            .cloned()
            .collect()
    }

    /// Open a collection (zero-access handle on Windows).
    pub fn open(&self, entry: &HidEntry, timing: Timing) -> Result<DeviceSession, HidError> {
        Ok(Session::new(backend::Device::open(&entry.path)?, timing))
    }
}

/// A session on a real device.
pub type DeviceSession = Session<backend::Device>;

/// Name of running Razer software that may talk to the device at the same time.
pub fn conflicting_process() -> Option<&'static str> {
    CONFLICTING
        .iter()
        .copied()
        .find(|name| backend::process_running(name))
}

/// Feature-report I/O. Implemented by the backends; mocked in tests.
pub trait FeatureIo {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError>;
    /// Fill `buf` (byte 0 = report ID on entry); returns bytes read.
    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError>;
}

/// Pacing and retry limits. Defaults follow the Viper V3 HyperSpeed manifest
/// (25 ms everywhere, 10 polls per send, 20 sends) — observed.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub between_out: Duration,
    pub out_to_in: Duration,
    pub between_in: Duration,
    pub max_in: u32,
    pub max_out: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            between_out: Duration::from_millis(25),
            out_to_in: Duration::from_millis(25),
            between_in: Duration::from_millis(25),
            max_in: 10,
            max_out: 20,
        }
    }
}

/// An open device: serialises request/response exchanges and owns the
/// transaction counter.
pub struct Session<D> {
    io: D,
    timing: Timing,
    counter: TransactionCounter,
}

impl<D: FeatureIo> Session<D> {
    pub fn new(io: D, timing: Timing) -> Self {
        Self {
            io,
            timing,
            counter: TransactionCounter::default(),
        }
    }

    /// Send `request` and wait for its reply (Synapse's send algorithm):
    /// SET → wait → GET; poll while busy; resend on a transient failure or a
    /// reply that does not match; stop on a permanent failure status.
    pub fn execute(&mut self, request: &Request) -> Result<Report, HidError> {
        let report = request.encode(self.counter.next_id());
        let wire = report.to_feature_report();
        let mut last = String::from("none");
        // The last attempt ended with status Timeout: classify as NoResponse.
        let mut timed_out = false;
        for attempt in 0..self.timing.max_out {
            if attempt > 0 {
                std::thread::sleep(self.timing.between_out);
            }
            tracing::debug!(?report, attempt, "send");
            timed_out = false;
            if let Err(e) = self.io.send_feature(&wire) {
                last = e.to_string();
                continue;
            }
            std::thread::sleep(self.timing.out_to_in);
            match self.poll(&report) {
                Outcome::Done(reply) => return Ok(reply),
                Outcome::Fail(status) => return Err(HidError::Status(status)),
                Outcome::Resend(reason) => last = reason,
                Outcome::TimedOut => {
                    last = format!("status {:?}", Status::Timeout);
                    timed_out = true;
                }
            }
        }
        if timed_out {
            return Err(HidError::NoResponse {
                attempts: self.timing.max_out,
            });
        }
        Err(HidError::Exhausted {
            attempts: self.timing.max_out,
            last,
        })
    }

    /// Read replies to one SET until it is done, failed, or needs a resend.
    fn poll(&self, request: &Report) -> Outcome {
        for poll in 0..self.timing.max_in {
            if poll > 0 {
                std::thread::sleep(self.timing.between_in);
            }
            let mut buf = [0u8; FEATURE_REPORT_LEN];
            let reply = match self.io.get_feature(&mut buf) {
                Err(e) => return Outcome::Resend(e.to_string()),
                Ok(n) if n < 9 => return Outcome::Resend(format!("short read ({n} bytes)")),
                Ok(_) => match Report::from_feature_report(&buf) {
                    Ok(r) => r,
                    Err(e) => return Outcome::Resend(e.to_string()),
                },
            };
            tracing::debug!(?reply, poll, "recv");
            if !reply.answers(request) {
                return Outcome::Resend("reply does not match request".into());
            }
            match reply.status().disposition() {
                Disposition::Done => return Outcome::Done(reply),
                Disposition::Poll => {}
                Disposition::Resend if reply.status() == Status::Timeout => {
                    return Outcome::TimedOut;
                }
                Disposition::Resend => {
                    return Outcome::Resend(format!("status {:?}", reply.status()));
                }
                Disposition::Fail => return Outcome::Fail(reply.status()),
            }
        }
        Outcome::Resend("device stayed busy".into())
    }
}

enum Outcome {
    Done(Report),
    Resend(String),
    /// Resend because the reply said Timeout (kept apart for [`HidError::NoResponse`]).
    TimedOut,
    Fail(Status),
}

#[cfg(test)]
mod tests;
