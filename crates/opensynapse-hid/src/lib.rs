//! hidapi transport and device sessions. See README.md.
//!
//! Transport facts: `vault/Protocol/Windows Transport.md`. Retry/busy rules:
//! `vault/Protocol/Report Format 2.5.md`. This crate never builds report bytes
//! itself; it moves [`opensynapse_protocol::Report`]s.

use std::ffi::CString;
use std::time::Duration;

use opensynapse_device_registry::RAZER_VID;
use opensynapse_protocol::{
    Disposition, FEATURE_REPORT_LEN, ProtocolError, Report, Request, Status, TransactionCounter,
};

mod process;
pub use process::conflicting_process;

/// Usage page / usage of the collection that carries the 91-byte feature report.
pub const CONTROL_USAGE_PAGE: u16 = 0x0001;
pub const CONTROL_USAGE: u16 = 0x0002;
/// Interface Synapse claims for the Viper V3 HyperSpeed (and the 2.5 default).
pub const DEFAULT_INTERFACE: i32 = 0;

#[derive(Debug, thiserror::Error)]
pub enum HidError {
    #[error("HID error: {0}")]
    Hid(#[from] hidapi::HidError),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("device answered {0:?}")]
    Status(Status),
    #[error("no answer after {attempts} attempts (last problem: {last})")]
    Exhausted { attempts: u32, last: String },
}

/// One HID collection of a Razer device, as enumerated.
#[derive(Clone, Debug)]
pub struct HidEntry {
    pub vid: u16,
    pub pid: u16,
    pub interface: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub path: CString,
    pub product: Option<String>,
}

impl HidEntry {
    /// True for the collection that carries the protocol 2.5 feature report.
    pub fn is_control(&self, interface: i32) -> bool {
        self.interface == interface
            && self.usage_page == CONTROL_USAGE_PAGE
            && self.usage == CONTROL_USAGE
    }
}

/// Enumerated Razer HID collections plus the hidapi context to open them.
pub struct Transport {
    api: hidapi::HidApi,
}

impl Transport {
    /// Enumerate HID collections. hidapi 2.6 always scans every HID device in
    /// `HidApi::new()` (its discovery switch only affects libusb), so this is the
    /// single scan; [`Transport::entries`] filters to Razer afterwards.
    pub fn new() -> Result<Self, HidError> {
        Ok(Self {
            api: hidapi::HidApi::new()?,
        })
    }

    /// Razer (VID 0x1532) collections.
    pub fn entries(&self) -> Vec<HidEntry> {
        self.api
            .device_list()
            .filter(|d| d.vendor_id() == RAZER_VID)
            .map(|d| HidEntry {
                vid: d.vendor_id(),
                pid: d.product_id(),
                interface: d.interface_number(),
                usage_page: d.usage_page(),
                usage: d.usage(),
                path: d.path().to_owned(),
                product: d.product_string().map(str::to_owned),
            })
            .collect()
    }

    /// Control collections (usage page 1 / usage 2 on `interface`), one per path.
    pub fn control_entries(&self, interface: i32) -> Vec<HidEntry> {
        let mut v: Vec<_> = self
            .entries()
            .into_iter()
            .filter(|e| e.is_control(interface))
            .collect();
        v.sort_by(|a, b| (a.pid, &a.path).cmp(&(b.pid, &b.path)));
        v.dedup_by(|a, b| a.path == b.path);
        v
    }

    /// Open a collection. hidapi falls back to a zero-access handle, which is
    /// enough for feature reports on the OS-owned mouse collection.
    pub fn open(
        &self,
        entry: &HidEntry,
        timing: Timing,
    ) -> Result<Session<hidapi::HidDevice>, HidError> {
        let dev = self.api.open_path(&entry.path)?;
        Ok(Session::new(dev, timing))
    }
}

/// A session on a real hidapi device.
pub type DeviceSession = Session<hidapi::HidDevice>;

/// Feature-report I/O. Implemented for `hidapi::HidDevice`; mocked in tests.
pub trait FeatureIo {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError>;
    /// Fill `buf` (byte 0 = report ID on entry); returns bytes read.
    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError>;
}

impl FeatureIo for hidapi::HidDevice {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError> {
        Ok(self.send_feature_report(buf)?)
    }

    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError> {
        Ok(self.get_feature_report(buf)?)
    }
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
        for attempt in 0..self.timing.max_out {
            if attempt > 0 {
                std::thread::sleep(self.timing.between_out);
            }
            tracing::debug!(?report, attempt, "send");
            if let Err(e) = self.io.send_feature(&wire) {
                last = e.to_string();
                continue;
            }
            std::thread::sleep(self.timing.out_to_in);
            match self.poll(&report) {
                Outcome::Done(reply) => return Ok(reply),
                Outcome::Fail(status) => return Err(HidError::Status(status)),
                Outcome::Resend(reason) => last = reason,
            }
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
    Fail(Status),
}

#[cfg(test)]
mod tests;
