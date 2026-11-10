//! Input reports (device → host) from one HID collection, read on a dedicated
//! thread that blocks in the kernel. No polling: the thread waits on "a report
//! arrived" or "stop requested", whichever comes first.
//!
//! Used for hardware events (`vault/Protocol/Hardware Events.md`). Windows only
//! for now (`windows_input.rs`); other platforms return `Unsupported`.

use crate::HidError;

/// Called with every input report (byte 0 = report ID, as the OS delivers it).
/// Return `false` to stop reading.
pub type OnReport = Box<dyn FnMut(&[u8]) -> bool + Send>;

/// A running reader thread. [`ReaderHandle::stop`] (or dropping the handle)
/// cancels the pending read and joins the thread.
pub struct ReaderHandle {
    #[cfg(windows)]
    inner: Option<crate::windows_input::Reader>,
    #[cfg(not(windows))]
    _priv: (),
}

impl ReaderHandle {
    /// Stop reading and wait for the thread. `Err` if the reader had already
    /// ended with an error (e.g. the device was unplugged).
    pub fn stop(mut self) -> Result<(), HidError> {
        self.stop_inner()
    }

    /// True once the thread has ended on its own (error, device gone, or the
    /// callback returned `false`).
    pub fn is_finished(&self) -> bool {
        #[cfg(windows)]
        {
            self.inner.as_ref().is_none_or(|r| r.is_finished())
        }
        #[cfg(not(windows))]
        {
            true
        }
    }

    fn stop_inner(&mut self) -> Result<(), HidError> {
        #[cfg(windows)]
        {
            match self.inner.take() {
                Some(r) => r.stop(),
                None => Ok(()),
            }
        }
        #[cfg(not(windows))]
        {
            Ok(())
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
/// reading (the OS holds mouse/keyboard collections exclusively) or has no
/// input report.
pub fn spawn(
    path: &str,
    on_report: impl FnMut(&[u8]) -> bool + Send + 'static,
) -> Result<ReaderHandle, HidError> {
    #[cfg(windows)]
    {
        let r = crate::windows_input::Reader::spawn_hid(path, Box::new(on_report))?;
        Ok(ReaderHandle { inner: Some(r) })
    }
    #[cfg(not(windows))]
    {
        let _ = (path, on_report);
        Err(HidError::Io(std::io::Error::from(
            std::io::ErrorKind::Unsupported,
        )))
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    #[test]
    fn unsupported_elsewhere() {
        assert!(super::spawn("x", |_| true).is_err());
    }
}
