//! Foreground-application watch for per-app profiles
//! (`vault/Decisions/ADR-0007 Per-app profiles.md`).
//!
//! [`watch`] starts one thread that sleeps in the OS until the foreground window
//! changes and calls back with the new foreground executable's full path, once per
//! change of executable (alt-tab between two windows of one program is silent).
//! No polling, no timers. Dropping the [`Watch`] stops the thread.

use std::io;

#[cfg(windows)]
#[path = "foreground_windows.rs"]
mod imp;

/// Called on the watch thread with the new foreground executable (full path).
pub type OnForeground = Box<dyn FnMut(String) + Send>;

/// A running watch; drop stops it.
pub struct Watch {
    #[cfg(windows)]
    _inner: imp::Watch,
}

/// Start watching. The callback also gets the foreground executable at start.
pub fn watch(f: OnForeground) -> io::Result<Watch> {
    #[cfg(windows)]
    {
        Ok(Watch {
            _inner: imp::watch(f)?,
        })
    }
    #[cfg(not(windows))]
    {
        drop(f);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "foreground watch is Windows-only for now",
        ))
    }
}

/// Remembers the last executable and lets only changes through.
#[derive(Default)]
pub struct Dedup {
    last: Option<String>,
}

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
}
