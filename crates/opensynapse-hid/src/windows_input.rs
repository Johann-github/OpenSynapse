//! Windows input-report reader: overlapped `ReadFile` on a `GENERIC_READ` handle,
//! one thread per collection. The thread sleeps in `WaitForMultipleObjects` on
//! [read done, cancel]; `stop` signals the cancel event, the thread cancels the
//! pending read with `CancelIoEx`, waits for it to finish and closes the handle.
//! No timeouts, no polling.

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::io;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, GENERIC_READ, GetLastError, HANDLE,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OVERLAPPED, ReadFile};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects,
};

use crate::HidError;
use crate::input::OnReport;
use crate::windows::{caps_of, open_raw};

/// An owned kernel handle (file or event), closed on drop.
struct Owned(HANDLE);

// SAFETY: kernel handles may be used and closed from any thread; the read
// handle is only used by the reader thread, events are thread-safe objects.
unsafe impl Send for Owned {}
// SAFETY: as above; SetEvent/WaitFor* on a shared event handle are thread-safe.
unsafe impl Sync for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful Create* call and is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// Manual-reset event, initially not signalled.
fn event() -> io::Result<Owned> {
    // SAFETY: no security attributes, no name; returns null on failure.
    let h = unsafe { CreateEventW(null(), 1, 0, null()) };
    if h.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(Owned(h))
}

/// A running reader thread.
pub struct Reader {
    cancel: Arc<Owned>,
    thread: JoinHandle<Result<(), HidError>>,
}

impl Reader {
    /// Open a HID collection for overlapped reads, sized by its input report length.
    pub fn spawn_hid(path: &str, on_report: OnReport) -> Result<Self, HidError> {
        let file = Owned(open_raw(path, GENERIC_READ, FILE_FLAG_OVERLAPPED)?);
        let len = caps_of(file.0).map_or(0, |c| c.InputReportByteLength);
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "collection has no input report",
            )
            .into());
        }
        Self::spawn(file, usize::from(len), on_report)
    }

    /// Start the thread on an already opened, overlapped, readable handle.
    fn spawn(file: Owned, len: usize, on_report: OnReport) -> Result<Self, HidError> {
        let cancel = Arc::new(event()?);
        let done = event()?;
        let stop = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("hid-input".into())
            .spawn(move || read_loop(&file, &done, &stop, len, on_report))?;
        Ok(Self { cancel, thread })
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Cancel the pending read, join the thread, return how it ended.
    pub fn stop(self) -> Result<(), HidError> {
        // SAFETY: valid event handle, kept alive by the Arc.
        unsafe { SetEvent(self.cancel.0) };
        self.thread
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("input reader thread panicked").into()))
    }
}

/// Cancel the read that uses `ov` and wait until the kernel is done with it, so
/// `ov` and the buffer may be dropped afterwards.
fn cancel_and_drain(file: &Owned, ov: &OVERLAPPED) {
    let mut n = 0u32;
    // SAFETY: `file` is the handle the read was issued on; `ov` is its OVERLAPPED,
    // alive until GetOverlappedResult(wait = TRUE) returns.
    unsafe {
        CancelIoEx(file.0, ov);
        GetOverlappedResult(file.0, ov, &mut n, 1);
    }
}

fn read_loop(
    file: &Owned,
    done: &Owned,
    cancel: &Owned,
    len: usize,
    mut on_report: OnReport,
) -> Result<(), HidError> {
    let mut buf = vec![0u8; len];
    let waits = [done.0, cancel.0];
    loop {
        // SAFETY: OVERLAPPED is plain data; all-zero (offset 0) is the documented start state.
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = done.0;
        // SAFETY: valid event handle.
        unsafe { ResetEvent(done.0) };
        // SAFETY: `buf` and `ov` outlive the I/O: every exit path below either sees
        // it complete or cancels it and waits for completion (`cancel_and_drain`).
        let ok = unsafe {
            ReadFile(
                file.0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                null_mut(),
                &mut ov,
            )
        };
        if ok == 0 {
            // SAFETY: reads the calling thread's last-error value.
            let e = unsafe { GetLastError() };
            if e != ERROR_IO_PENDING {
                return Err(io::Error::from_raw_os_error(e as i32).into());
            }
        }
        // SAFETY: two valid event handles; blocks in the kernel until one is signalled.
        let w = unsafe { WaitForMultipleObjects(2, waits.as_ptr(), 0, INFINITE) };
        if w != WAIT_OBJECT_0 {
            let err = (w != WAIT_OBJECT_0 + 1).then(io::Error::last_os_error);
            cancel_and_drain(file, &ov);
            return err.map_or(Ok(()), |e| Err(e.into()));
        }
        let mut n = 0u32;
        // SAFETY: the read on `ov` has completed (its event is signalled).
        if unsafe { GetOverlappedResult(file.0, &ov, &mut n, 0) } == 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(ERROR_OPERATION_ABORTED as i32) {
                return Ok(());
            }
            return Err(e.into());
        }
        if !on_report(&buf[..n as usize]) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use windows_sys::Win32::Storage::FileSystem::{PIPE_ACCESS_OUTBOUND, WriteFile};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use super::*;

    /// A named pipe stands in for a HID collection: same overlapped ReadFile path.
    fn pipe(tag: &str) -> (Owned, String) {
        let name = format!(
            r"\\.\pipe\opensynapse-hid-input-test-{tag}-{}",
            std::process::id()
        );
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: NUL-terminated name; returns INVALID_HANDLE_VALUE on failure.
        let h = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_OUTBOUND,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                null(),
            )
        };
        assert_ne!(h, windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE);
        (Owned(h), name)
    }

    fn reader(name: &str, server: &Owned) -> (Reader, Receiver<Vec<u8>>) {
        let file = Owned(open_raw(name, GENERIC_READ, FILE_FLAG_OVERLAPPED).unwrap());
        // SAFETY: valid pipe handle; the client is already connected, so this
        // returns at once (ERROR_PIPE_CONNECTED).
        unsafe { ConnectNamedPipe(server.0, null_mut()) };
        let (tx, rx) = channel();
        let r = Reader::spawn(
            file,
            8,
            Box::new(move |b: &[u8]| tx.send(b.to_vec()).is_ok() && b != [0xFF]),
        )
        .unwrap();
        (r, rx)
    }

    fn write(server: &Owned, data: &[u8]) {
        let mut n = 0u32;
        // SAFETY: valid pipe handle and buffer; synchronous write.
        let ok = unsafe {
            WriteFile(
                server.0,
                data.as_ptr(),
                data.len() as u32,
                &mut n,
                null_mut(),
            )
        };
        assert!(ok != 0 && n as usize == data.len());
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn delivers_reports_and_stops_on_cancel() {
        let (server, name) = pipe("cancel");
        let (r, rx) = reader(&name, &server);
        write(&server, &[5, 0x02, 0x03]);
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), vec![5, 0x02, 0x03]);
        write(&server, &[5, 0x31]);
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), vec![5, 0x31]);
        // A read is pending now; stop must cancel it and join.
        assert!(!r.is_finished());
        r.stop().unwrap();
    }

    #[test]
    fn callback_can_end_the_reader() {
        let (server, name) = pipe("callback");
        let (r, rx) = reader(&name, &server);
        write(&server, &[0xFF]);
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), vec![0xFF]);
        r.stop().unwrap();
    }

    #[test]
    fn closed_source_ends_with_error() {
        let (server, name) = pipe("closed");
        let (r, rx) = reader(&name, &server);
        drop(server);
        // The thread drops its sender when it ends.
        assert!(rx.recv_timeout(WAIT).is_err());
        assert!(r.stop().is_err(), "broken pipe is reported");
    }

    #[test]
    fn missing_path_fails_to_open() {
        let r = Reader::spawn_hid(
            r"\\?\hid#vid_0000&pid_0000#nonexistent#{x}",
            Box::new(|_| true),
        );
        assert!(r.is_err());
    }
}
