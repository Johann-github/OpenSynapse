//! Linux foreground watch, X11 only: the agent's own X connection (`x11rb`, pure
//! Rust, no libX11/libxcb) selects `PropertyChangeMask` on the root window and
//! reacts to `PropertyNotify` for `_NET_ACTIVE_WINDOW` (EWMH). The active window's
//! `_NET_WM_PID` → `/proc/<pid>/exe` gives the executable. One thread sleeps in
//! `poll([X socket, eventfd], -1)`; no timer. Wayland and headless sessions get
//! `Unsupported`. Decision: `vault/Decisions/ADR-0013 Foreground watch on Linux and macOS.md`.

#![allow(unsafe_code)] // eventfd/poll FFI only; every block states its invariant.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::thread::JoinHandle;

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;

use super::x11::{self, Session};
use super::{Dedup, OnForeground};

/// The running watch thread. Drop wakes it through the eventfd and joins it, so a
/// callback that is running at that moment finishes first.
pub struct Watch {
    stop: OwnedFd,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watch {
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

fn xerr(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("X11: {e}"))
}

pub fn watch(f: OnForeground) -> io::Result<Watch> {
    let var = |k: &str| std::env::var(k).ok();
    match x11::session(
        var("WAYLAND_DISPLAY").as_deref(),
        var("XDG_SESSION_TYPE").as_deref(),
        var("DISPLAY").as_deref(),
    ) {
        Session::X11 => {}
        Session::Wayland => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Wayland has no generic foreground-window API",
            ));
        }
        Session::Headless => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no X11 display (DISPLAY is not set)",
            ));
        }
    }
    let x = X::connect()?;

    // SAFETY: plain syscall; the result is checked.
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a fresh, open fd that nothing else owns.
    let stop = unsafe { OwnedFd::from_raw_fd(raw) };

    let stop_fd = stop.as_raw_fd();
    let thread = std::thread::Builder::new()
        .name("foreground".into())
        .stack_size(256 * 1024)
        .spawn(move || run(&x, stop_fd, f))?;
    Ok(Watch {
        stop,
        thread: Some(thread),
    })
}

/// The agent's X connection and the atoms it needs.
struct X {
    conn: RustConnection,
    root: Window,
    active: Atom,
    pid: Atom,
}

impl X {
    /// Connect to `DISPLAY` (with `XAUTHORITY` cookies, done by x11rb) and select
    /// property changes on the root window. Event masks are per client, so this
    /// does not affect the window manager or other clients.
    fn connect() -> io::Result<X> {
        let (conn, screen) = x11rb::connect(None).map_err(xerr)?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .ok_or_else(|| xerr("the display has no such screen"))?
            .root;
        let intern = |name: &[u8]| -> io::Result<Atom> {
            Ok(conn
                .intern_atom(false, name)
                .map_err(xerr)?
                .reply()
                .map_err(xerr)?
                .atom)
        };
        let active = intern(b"_NET_ACTIVE_WINDOW")?;
        let pid = intern(b"_NET_WM_PID")?;
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(xerr)?
        .check()
        .map_err(xerr)?;
        Ok(X {
            conn,
            root,
            active,
            pid,
        })
    }

    /// First CARD32/WINDOW item of `prop` on `window`; `None` when it is missing or
    /// the window is gone (an X error comes back as `Err`, it does not abort).
    fn prop32(&self, window: Window, prop: Atom, ty: AtomEnum) -> Option<u32> {
        let r = self
            .conn
            .get_property(false, window, prop, ty, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        x11::card32(r.format, &r.value)
    }

    /// The active window's executable; `None` without an active window, without
    /// `_NET_WM_PID`, or for a process this user may not inspect.
    fn active_exe(&self) -> Option<String> {
        let window = self.prop32(self.root, self.active, AtomEnum::WINDOW)?;
        let pid = self.prop32(window, self.pid, AtomEnum::CARDINAL)?;
        let target = std::fs::read_link(x11::proc_exe(pid)).ok()?;
        x11::exe_from_link(&target.to_string_lossy())
    }
}

/// Thread body: report the app in front, then sleep in `poll` until X sends an
/// event or the stop signal arrives. `stop` stays open until this returns.
fn run(x: &X, stop: RawFd, mut f: OnForeground) {
    let mut dedup = Dedup::default();
    let mut report = |exe: Option<String>| {
        if let Some(exe) = exe
            && dedup.changed(&exe)
        {
            f(exe);
        }
    };
    report(x.active_exe());
    if let Err(e) = x.conn.flush() {
        tracing::warn!(error = %e, "X11 flush failed; foreground watch stops");
        return;
    }
    let xfd = x.conn.stream().as_raw_fd();
    loop {
        // Drain events x11rb has already read: waiting for a reply also reads
        // events, which `poll` on the socket would then never announce.
        let mut changed = false;
        loop {
            match x.conn.poll_for_event() {
                Ok(Some(Event::PropertyNotify(ev))) => {
                    changed |= ev.window == x.root && ev.atom == x.active;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!(error = %e, "X11 connection lost; foreground watch stops");
                    return;
                }
            }
        }
        if changed {
            // A burst of changes costs one lookup; its replies may queue new
            // events, so drain again before sleeping.
            report(x.active_exe());
            continue;
        }

        let mut fds = [
            libc::pollfd {
                fd: xfd,
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
            tracing::warn!(error = %e, "foreground poll failed; watch stops");
            return;
        }
        if fds[1].revents != 0 {
            return;
        }
        let revents = fds[0].revents;
        if revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            // The X server went away (session end); nothing useful is left to read,
            // and returning here also rules out spinning on a hung-up socket.
            tracing::warn!(revents, "X11 socket closed; foreground watch stops");
            return;
        }
        // POLLIN: the drain at the top of the loop reads it.
    }
}
