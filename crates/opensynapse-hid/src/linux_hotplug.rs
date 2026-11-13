//! Linux hotplug: a netlink `NETLINK_KOBJECT_UEVENT` socket on the udev multicast
//! group, read by one thread that sleeps in `poll(2)` without a timeout. An eventfd
//! in the same `poll` set wakes it for shutdown. No timers, no polling loop.
//! Decision: `vault/Decisions/ADR-0010 Linux hotplug.md`; public API in `hotplug.rs`.

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::thread::JoinHandle;

use crate::HidError;
use crate::hotplug::Callback;
use crate::uevent;

/// Netlink multicast group bitmask of the udev daemon's re-broadcast (1 = raw kernel
/// events). udev sends after its rules ran, so the node already has its permissions
/// (`uaccess`) when the agent rescans.
const UDEV_GROUP: u32 = 2;
/// Large enough for one udev message (properties of a hidraw node are < 2 KiB).
const BUF_LEN: usize = 16 * 1024;

/// A running watcher thread. Drop wakes it through the eventfd and joins it, so a
/// callback that is running at that moment finishes first.
pub struct Watcher {
    stop: OwnedFd,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let one: u64 = 1;
        // SAFETY: `stop` is our open eventfd; we pass a valid pointer to 8 readable
        // bytes, which is exactly what an eventfd write takes.
        let _ = unsafe { libc::write(self.stop.as_raw_fd(), (&raw const one).cast(), 8) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        // `stop` closes after the join, so the thread never sees a stale fd.
    }
}

fn last_error() -> HidError {
    HidError::Io(io::Error::last_os_error())
}

/// Open the uevent socket and start the watcher thread for vendor `vid`.
pub fn spawn(vid: u16, f: Callback) -> Result<Watcher, HidError> {
    // SAFETY: plain syscall with constant arguments; the result is checked.
    let raw = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_KOBJECT_UEVENT,
        )
    };
    if raw < 0 {
        return Err(last_error());
    }
    // SAFETY: `raw` is a fresh, open fd that nothing else owns.
    let sock = unsafe { OwnedFd::from_raw_fd(raw) };

    // SAFETY: sockaddr_nl is plain old data; all-zero is a valid value (pid 0 lets
    // the kernel pick the port id).
    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    addr.nl_groups = UDEV_GROUP;
    // SAFETY: `addr` is a valid sockaddr_nl and the length matches its size.
    let r = unsafe {
        libc::bind(
            sock.as_raw_fd(),
            (&raw const addr).cast(),
            size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if r < 0 {
        return Err(last_error());
    }

    // SAFETY: plain syscall; the result is checked.
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
    if raw < 0 {
        return Err(last_error());
    }
    // SAFETY: `raw` is a fresh, open fd that nothing else owns.
    let stop = unsafe { OwnedFd::from_raw_fd(raw) };

    let stop_fd = stop.as_raw_fd();
    let thread = std::thread::Builder::new()
        .name("hotplug".into())
        .spawn(move || run(&sock, stop_fd, vid, &f))?;
    Ok(Watcher {
        stop,
        thread: Some(thread),
    })
}

/// Thread body: sleep in `poll` until a datagram or the stop signal arrives.
/// `stop` stays open until this returns (Watcher::drop joins before closing it).
fn run(sock: &OwnedFd, stop: RawFd, vid: u16, f: &Callback) {
    let mut buf = vec![0u8; BUF_LEN];
    loop {
        let mut fds = [
            libc::pollfd {
                fd: sock.as_raw_fd(),
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
            tracing::warn!(error = %e, "hotplug poll failed; watcher stops");
            return;
        }
        if fds[1].revents != 0 {
            return;
        }
        let revents = fds[0].revents;
        if revents & (libc::POLLHUP | libc::POLLNVAL) != 0 {
            tracing::warn!(revents, "hotplug socket closed; watcher stops");
            return;
        }
        if revents == 0 {
            continue;
        }
        // POLLIN, or POLLERR after a receive-buffer overrun (recv then reports ENOBUFS).
        // SAFETY: `buf` is valid for `buf.len()` writable bytes; non-blocking receive.
        let r = unsafe {
            libc::recv(
                sock.as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if r < 0 {
            let e = io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EAGAIN | libc::EINTR) => {}
                Some(libc::ENOBUFS) => tracing::warn!("hotplug: uevents lost (buffer overrun)"),
                _ => {
                    tracing::warn!(error = %e, "hotplug recv failed; watcher stops");
                    return;
                }
            }
            continue;
        }
        let len = usize::try_from(r).unwrap_or(0).min(buf.len());
        if let Some(ev) = uevent::parse(&buf[..len], vid) {
            f(ev);
        }
    }
}
