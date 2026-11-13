//! Foreground-application watch for per-app profiles
//! (`vault/Decisions/ADR-0007 Per-app profiles.md`).
//!
//! [`watch`] starts one thread that sleeps in the OS until the foreground window
//! changes and calls back with the new foreground executable's full path, once per
//! change of executable (alt-tab between two windows of one program is silent).
//! No polling, no timers. Dropping the [`Watch`] stops the thread.
//!
//! Platforms: Windows (WinEvent hook) and Linux X11 (`_NET_ACTIVE_WINDOW`,
//! `vault/Decisions/ADR-0013 Foreground watch on Linux and macOS.md`). Wayland,
//! headless Linux and macOS (decided, pending a Mac to test on) get `Unsupported`.

use std::io;

#[cfg(windows)]
#[path = "foreground_windows.rs"]
mod imp;

#[cfg(target_os = "linux")]
#[path = "foreground_linux.rs"]
mod imp;

/// Pure X11 helpers: used by the Linux watch, unit-tested everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[path = "foreground_x11.rs"]
mod x11;

/// Called on the watch thread with the new foreground executable (full path).
pub type OnForeground = Box<dyn FnMut(String) + Send>;

/// A running watch; drop stops it.
pub struct Watch {
    #[cfg(any(windows, target_os = "linux"))]
    _inner: imp::Watch,
}

/// Start watching. The callback also gets the foreground executable at start.
pub fn watch(f: OnForeground) -> io::Result<Watch> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        Ok(Watch {
            _inner: imp::watch(f)?,
        })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        drop(f);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "foreground watch is not implemented on this platform yet (ADR-0013)",
        ))
    }
}

/// Remembers the last executable and lets only changes through. Used by the
/// platform watchers (Windows, Linux X11).
#[derive(Default)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub struct Dedup {
    last: Option<String>,
}

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
impl Dedup {
    /// `true` when `exe` differs from the previous one (which it then becomes).
    pub fn changed(&mut self, exe: &str) -> bool {
        if self.last.as_deref() == Some(exe) {
            return false;
        }
        self.last = Some(exe.to_owned());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_passes_changes_only() {
        let mut d = Dedup::default();
        assert!(d.changed("a.exe"));
        assert!(!d.changed("a.exe"));
        assert!(d.changed("b.exe"));
        assert!(d.changed("a.exe"));
    }

    /// Starts and stops the real hook (no device access). The callback may or may
    /// not fire, depending on whether the test runner has a foreground window.
    #[cfg(windows)]
    #[test]
    fn watch_starts_and_stops() {
        let w = watch(Box::new(|_| {})).expect("hook the foreground event");
        drop(w);
    }

    /// Linux: starts and stops the X11 watch when a display is reachable; without
    /// one (CI, Wayland) it must fail cleanly instead of hanging or panicking.
    #[cfg(target_os = "linux")]
    #[test]
    fn watch_starts_and_stops_or_reports_unsupported() {
        match watch(Box::new(|_| {})) {
            Ok(w) => drop(w),
            Err(e) => eprintln!("no foreground watch here: {e}"),
        }
    }
}
