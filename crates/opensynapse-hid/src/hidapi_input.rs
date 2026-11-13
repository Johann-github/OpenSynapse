//! Linux/macOS input-report reader on hidapi: one thread per collection, moving
//! the `HidDevice` into it.
//!
//! Stop mechanism (trade-off, read this before changing it): hidapi has no way
//! to unblock a pending read from another thread. `HidDevice` is `Send` but not
//! `Sync`, so it cannot be shared with the thread that wants to stop the reader,
//! and closing it under a blocked `hid_read` would be a use-after-free in the C
//! library. So the thread reads with `read_timeout(STOP_CHECK_MS)` and checks an
//! atomic stop flag between reads. When no report arrives, the thread wakes once
//! per second, looks at one flag and blocks again. That is a timed wakeup, not
//! polling the device (no I/O is issued for it, the kernel read stays the
//! waiting mechanism), but it is not the zero-wakeup idle of the Windows reader
//! either. `stop` therefore takes up to `STOP_CHECK_MS`. A fully blocking reader
//! would need the raw hidraw fd + `poll` on a self-pipe (Linux) or a CFRunLoop
//! source (macOS) outside hidapi; not worth it until measured.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use crate::HidError;
use crate::hidapi_backend::{Device, io_err};
use crate::input::{OnReport, READ_BUF_LEN, STOP_CHECK_MS, report_id_offset, uses_report_ids};

/// hidapi's recommended report descriptor buffer (`HID_API_MAX_REPORT_DESCRIPTOR_SIZE`).
const DESCRIPTOR_LEN: usize = 4096;

/// A running reader thread.
pub struct Reader {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Result<(), HidError>>,
}

impl Reader {
    /// Open a HID collection through hidapi and start reading it.
    pub fn spawn_hid(path: &str, on_report: OnReport) -> Result<Self, HidError> {
        let dev = Device::open(path)?.0;
        let numbered = numbered(&dev);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("hid-input".into())
            .spawn(move || read_loop(&dev, numbered, &flag, on_report))?;
        Ok(Self { stop, thread })
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Ask the thread to stop, join it (up to `STOP_CHECK_MS`), return how it ended.
    pub fn stop(self) -> Result<(), HidError> {
        self.stop.store(true, Ordering::Release);
        self.thread
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("input reader thread panicked").into()))
    }
}

/// Whether the device sends numbered reports (report ID in byte 0). If the
/// descriptor cannot be read, assume numbered: Razer event collections use
/// report ID 5 (`vault/Protocol/Hardware Events.md`), and then bytes pass
/// through unchanged.
fn numbered(dev: &hidapi::HidDevice) -> bool {
    let mut desc = vec![0u8; DESCRIPTOR_LEN];
    match dev.get_report_descriptor(&mut desc) {
        Ok(n) => uses_report_ids(&desc[..n.min(desc.len())]),
        Err(e) => {
            tracing::debug!(error = %e, "no report descriptor; assuming numbered reports");
            true
        }
    }
}

fn read_loop(
    dev: &hidapi::HidDevice,
    numbered: bool,
    stop: &AtomicBool,
    mut on_report: OnReport,
) -> Result<(), HidError> {
    let off = report_id_offset(numbered);
    // Byte 0 stays 0 for unnumbered reports (see `report_id_offset`).
    let mut buf = vec![0u8; off + READ_BUF_LEN];
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let n = dev
            .read_timeout(&mut buf[off..], STOP_CHECK_MS)
            .map_err(io_err)?;
        if n == 0 {
            continue; // timed out: only the stop check above
        }
        if !on_report(&buf[..off + n]) {
            return Ok(());
        }
    }
}
