//! Linux input-report reader on the raw hidraw node: one thread per collection,
//! sleeping in `poll(2)` without a timeout on the hidraw fd and an eventfd. Stop
//! writes the eventfd and joins. No timers, no wakeups while idle (same pattern
//! as `linux_hotplug.rs`, `vault/Decisions/ADR-0010 Linux hotplug.md`).
//!
//! hidapi is bypassed here because it cannot unblock a read from another thread
//! (see `hidapi_input.rs`, still used on macOS). The path is the one hidapi's
//! hidraw backend enumerates (`/dev/hidrawN`). A hidraw `read` returns one report
//! per call: with the report ID first for numbered reports, data only for
//! unnumbered ones, the same bytes hidapi's hidraw backend hands out.

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::thread::JoinHandle;

use crate::HidError;
use crate::input::{
    OnReport, READ_BUF_LEN, hidraw_descriptor_path, report_id_offset, uses_report_ids,
};

/// A running reader thread. `stop` and Drop wake it through the eventfd and join it,
/// so a callback that is running at that moment finishes first.
pub struct Reader {
    stop: OwnedFd,
    thread: Option<JoinHandle<Result<(), HidError>>>,
}

impl Reader {
    /// Open the hidraw node at `path` read-only and start reading it.
    pub fn spawn_hid(path: &str, on_report: OnReport) -> Result<Self, HidError> {
        // std opens with O_CLOEXEC; O_NONBLOCK so a spurious POLLIN cannot block `read`.
        let file = File::options()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;
        let numbered = numbered(path);

        // SAFETY: plain syscall; the result is checked.
        let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
        if raw < 0 {
            return Err(last_error());
        }
        // SAFETY: `raw` is a fresh, open fd that nothing else owns.
        let stop = unsafe { OwnedFd::from_raw_fd(raw) };

        let stop_fd = stop.as_raw_fd();
        let thread = std::thread::Builder::new()
            .name("hid-input".into())
            .spawn(move || read_loop(&file, stop_fd, numbered, on_report))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Wake the thread, join it at once, return how it ended.
    pub fn stop(mut self) -> Result<(), HidError> {
        self.signal();
        match self.thread.take() {
            Some(t) => t
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("input reader thread panicked").into())),
            None => Ok(()),
        }
    }

    fn signal(&self) {
        let one: u64 = 1;
        // SAFETY: `stop` is our open eventfd; we pass a valid pointer to 8 readable
        // bytes, which is exactly what an eventfd write takes.
        let _ = unsafe { libc::write(self.stop.as_raw_fd(), (&raw const one).cast(), 8) };
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            self.signal();
            let _ = t.join();
        }
        // `stop` closes after the join, so the thread never sees a stale fd.
    }
}

fn last_error() -> HidError {
    HidError::Io(io::Error::last_os_error())
}

/// Whether the interface sends numbered reports (report ID in byte 0), from the
/// report descriptor the kernel exposes in sysfs (the same bytes as the
/// `HIDIOCGRDESC` ioctl hidapi uses). If it cannot be read, assume numbered, as
/// the hidapi reader does: Razer event collections use report ID 5
/// (`vault/Protocol/Hardware Events.md`), and then bytes pass through unchanged.
fn numbered(path: &str) -> bool {
    let read = hidraw_descriptor_path(path)
        .ok_or_else(|| io::Error::other("not a /dev/hidrawN path"))
        .and_then(std::fs::read);
    match read {
        Ok(desc) => uses_report_ids(&desc),
        Err(e) => {
            tracing::debug!(error = %e, path, "no report descriptor; assuming numbered reports");
            true
        }
    }
}

/// Thread body: sleep in `poll` until a report, a hang-up or the stop signal.
/// `stop` stays open until this returns (`Reader` joins before closing it).
fn read_loop(
    mut file: &File,
    stop: RawFd,
    numbered: bool,
    mut on_report: OnReport,
) -> Result<(), HidError> {
    let off = report_id_offset(numbered);
    // Byte 0 stays 0 for unnumbered reports (see `report_id_offset`).
    let mut buf = vec![0u8; off + READ_BUF_LEN];
    loop {
        let mut fds = [
            libc::pollfd {
                fd: file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stop,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: `fds` is a valid array of 2 pollfd; timeout -1 blocks until an event.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.into());
        }
        if fds[1].revents != 0 {
            return Ok(());
        }
        let revents = fds[0].revents;
        if revents & libc::POLLIN != 0 {
            // One report per read. A gone device surfaces here as an error
            // (ENODEV/EIO), which ends the reader like a hidapi read error.
            match file.read(&mut buf[off..]) {
                Ok(0) => return Err(gone(revents)),
                Ok(len) => {
                    if !on_report(&buf[..off + len]) {
                        return Ok(());
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e.into()),
            }
            continue;
        }
        if revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            // hidraw reports POLLERR|POLLHUP once the device is unplugged.
            return Err(gone(revents));
        }
    }
}

fn gone(revents: libc::c_short) -> HidError {
    HidError::Io(io::Error::other(format!(
        "hidraw device disconnected (poll revents {revents:#x})"
    )))
}
