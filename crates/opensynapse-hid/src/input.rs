//! Input reports (device → host) from one HID collection, read on a dedicated
//! thread that blocks in the kernel.
//!
//! Used for hardware events (`vault/Protocol/Hardware Events.md`).
//! - Windows (`windows_input.rs`): overlapped `ReadFile`; the thread waits on
//!   "a report arrived" or "stop requested". No timeouts at all.
//! - Linux (`linux_input.rs`): raw hidraw fd; the thread sleeps in `poll` on the
//!   fd and an eventfd that `stop` writes. No timeouts at all.
//! - macOS (`hidapi_input.rs`): hidapi `read_timeout`; without a report the
//!   thread wakes once per [`STOP_CHECK_MS`] only to see whether `stop` was asked
//!   for (hidapi cannot unblock a read from another thread). Open item: the one
//!   platform with an idle wakeup. Stage 3 of `vault/Architecture/Linux and macOS port.md`.

use crate::HidError;

#[cfg(all(not(windows), not(target_os = "linux")))]
use crate::hidapi_input::Reader;
#[cfg(target_os = "linux")]
use crate::linux_input::Reader;
#[cfg(windows)]
use crate::windows_input::Reader;

/// Called with every input report (byte 0 = report ID, as Windows delivers it;
/// 0 for a collection without numbered reports). Return `false` to stop reading.
pub type OnReport = Box<dyn FnMut(&[u8]) -> bool + Send>;

/// macOS (hidapi reader): longest wait in one hidapi read before the reader
/// thread checks its stop flag. Bounds how long [`ReaderHandle::stop`] may take.
#[cfg_attr(any(windows, target_os = "linux"), allow(dead_code))]
pub(crate) const STOP_CHECK_MS: i32 = 1000;

/// Linux/macOS: read buffer size. hidraw/hidapi do not tell the input report length
/// (Windows takes it from the collection's caps); a longer report would be cut
/// to this length. Razer event reports are short and a full-speed interrupt
/// packet is at most 64 bytes, so 1 KiB leaves ample room.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) const READ_BUF_LEN: usize = 1024;

/// True if a HID report descriptor declares a Report ID (global item, tag 8).
/// The HID spec makes report IDs all-or-nothing per interface, so one item is
/// enough. Walks the item structure, so a data byte equal to 0x85 does not count.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn uses_report_ids(desc: &[u8]) -> bool {
    let mut i = 0;
    while let Some(&prefix) = desc.get(i) {
        if prefix == 0xFE {
            // Long item: prefix, data size, tag, data.
            let size = desc.get(i + 1).copied().map_or(0, usize::from);
            i += 3 + size;
            continue;
        }
        if prefix & 0xFC == 0x84 {
            return true;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            s => usize::from(s),
        };
        i += 1 + size;
    }
    false
}

/// Linux: sysfs file with the report descriptor of hidraw node `dev_path`
/// (`/dev/hidrawN` → `/sys/class/hidraw/hidrawN/device/report_descriptor`).
/// `None` if the path is not a hidraw node.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn hidraw_descriptor_path(dev_path: &str) -> Option<String> {
    let name = dev_path.strip_prefix("/dev/")?;
    let n = name.strip_prefix("hidraw")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("/sys/class/hidraw/{name}/device/report_descriptor"))
}

/// Where in the read buffer the reader should write, so that byte 0 ends up as the
/// report ID like on Windows. hidraw/hidapi include the ID for numbered reports
/// (offset 0); for unnumbered ones it delivers data only, while Windows puts a
/// 0 in front, so byte 0 stays zero (offset 1).
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn report_id_offset(numbered: bool) -> usize {
    usize::from(!numbered)
}

/// A running reader thread. [`ReaderHandle::stop`] (or dropping the handle)
/// cancels the pending read and joins the thread.
pub struct ReaderHandle {
    inner: Option<Reader>,
}

impl ReaderHandle {
    /// Stop reading and wait for the thread. `Err` if the reader had already
    /// ended with an error (e.g. the device was unplugged). Windows and Linux
    /// return at once; macOS may take up to [`STOP_CHECK_MS`].
    pub fn stop(mut self) -> Result<(), HidError> {
        self.stop_inner()
    }

    /// True once the thread has ended on its own (error, device gone, or the
    /// callback returned `false`).
    pub fn is_finished(&self) -> bool {
        self.inner.as_ref().is_none_or(|r| r.is_finished())
    }

    fn stop_inner(&mut self) -> Result<(), HidError> {
        match self.inner.take() {
            Some(r) => r.stop(),
            None => Ok(()),
        }
    }
}

impl Drop for ReaderHandle {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

/// Open the collection at `path` for reading and call `on_report` with every
/// input report on a new thread. Fails if the collection cannot be opened for
/// reading (Windows: the OS holds mouse/keyboard collections exclusively) or has
/// no input report (detected on Windows only).
///
/// Linux difference: a hidraw node is a whole USB interface, so the callback
/// sees the input reports of every collection on that interface (dispatch on
/// byte 0); Windows delivers only the opened collection's reports.
pub fn spawn(
    path: &str,
    on_report: impl FnMut(&[u8]) -> bool + Send + 'static,
) -> Result<ReaderHandle, HidError> {
    let r = Reader::spawn_hid(path, Box::new(on_report))?;
    Ok(ReaderHandle { inner: Some(r) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Start of a typical vendor collection with report ID 5.
    const NUMBERED: &[u8] = &[
        0x06, 0x00, 0xFF, // Usage Page (vendor 0xFF00)
        0x09, 0x00, // Usage 0
        0xA1, 0x01, // Collection (Application)
        0x85, 0x05, // Report ID 5
        0x75, 0x08, // Report Size 8
        0x95, 0x0F, // Report Count 15
        0x81, 0x02, // Input
        0xC0, // End Collection
    ];

    #[test]
    fn finds_report_id_item() {
        assert!(uses_report_ids(NUMBERED));
    }

    #[test]
    fn unnumbered_descriptor() {
        let desc = [
            0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x75, 0x08, 0x95, 0x04, 0x81, 0x02, 0xC0,
        ];
        assert!(!uses_report_ids(&desc));
    }

    #[test]
    fn data_byte_0x85_is_not_an_item() {
        // Logical Maximum (2 bytes) = 0x0085, then a 4-byte item full of 0x85.
        let desc = [0x26, 0x85, 0x00, 0x27, 0x85, 0x85, 0x85, 0x85, 0xC0];
        assert!(!uses_report_ids(&desc));
    }

    #[test]
    fn skips_long_items() {
        // Long item with data bytes 0x85 0x01, then (second case) a real Report ID.
        assert!(!uses_report_ids(&[0xFE, 0x02, 0x10, 0x85, 0x01]));
        assert!(uses_report_ids(&[0xFE, 0x02, 0x10, 0x85, 0x01, 0x85, 0x02]));
    }

    #[test]
    fn truncated_descriptor_does_not_panic() {
        assert!(!uses_report_ids(&[]));
        assert!(!uses_report_ids(&[0x26]));
        assert!(!uses_report_ids(&[0xFE]));
        assert!(uses_report_ids(&[0x85]));
    }

    #[test]
    fn report_id_lands_in_byte_0() {
        // Numbered: hidapi already delivers [5, …].
        assert_eq!(report_id_offset(true), 0);
        // Unnumbered: data goes after a zero byte, like Windows' [0, …].
        assert_eq!(report_id_offset(false), 1);
    }

    #[test]
    fn hidraw_sysfs_descriptor() {
        assert_eq!(
            hidraw_descriptor_path("/dev/hidraw3").as_deref(),
            Some("/sys/class/hidraw/hidraw3/device/report_descriptor")
        );
        assert_eq!(
            hidraw_descriptor_path("/dev/hidraw12").as_deref(),
            Some("/sys/class/hidraw/hidraw12/device/report_descriptor")
        );
        // Not a hidraw node, or an attempt to escape sysfs.
        for p in [
            "/dev/hidraw",
            "/dev/hidraw1/../x",
            "/dev/input/event3",
            "hidraw0",
            "/dev/hidrawX",
            "",
        ] {
            assert_eq!(hidraw_descriptor_path(p), None, "{p}");
        }
    }

    #[test]
    fn missing_path_fails_to_open() {
        let path = if cfg!(windows) {
            r"\\?\hid#vid_0000&pid_0000#nonexistent#{x}"
        } else {
            "/dev/opensynapse-nonexistent-hidraw"
        };
        assert!(spawn(path, |_| true).is_err());
    }
}
