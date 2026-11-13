//! Pure parts of the Linux X11 foreground watch
//! (`vault/Decisions/ADR-0013 Foreground watch on Linux and macOS.md`): session
//! detection, EWMH property decoding and `/proc` path handling. No I/O, compiled on
//! every platform and unit-tested on Windows; the X11 connection and the thread
//! live in `foreground_linux.rs`.
//!
//! Executable names: the callback gets the target of `/proc/<pid>/exe`, the real
//! binary (e.g. `/usr/lib/firefox/firefox`, `/usr/share/code/code`). Rules match its
//! file name without case (`opensynapse_core::app_profiles::exe_matches`), so a rule
//! `firefox` works; a script's interpreter is reported (`/usr/bin/python3.12`).

/// What kind of graphical session the agent runs in.
#[derive(Debug, PartialEq, Eq)]
pub enum Session {
    /// Plain X11: `_NET_ACTIVE_WINDOW` on the root window tells the active window.
    X11,
    /// Wayland (possibly with XWayland): no generic API, the watch stays off.
    Wayland,
    /// No display at all (console, ssh, service).
    Headless,
}

/// Decide from `WAYLAND_DISPLAY`, `XDG_SESSION_TYPE` and `DISPLAY`. A Wayland
/// session wins over `DISPLAY`: XWayland only sees X11 clients, so native Wayland
/// apps in front would look like "no change" and keep a wrong profile.
pub fn session(
    wayland_display: Option<&str>,
    xdg_session_type: Option<&str>,
    display: Option<&str>,
) -> Session {
    let set = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());
    let wayland_type = xdg_session_type.is_some_and(|t| t.trim().eq_ignore_ascii_case("wayland"));
    if set(wayland_display) || wayland_type {
        Session::Wayland
    } else if set(display) {
        Session::X11
    } else {
        Session::Headless
    }
}

/// The first 32-bit item of a `GetProperty` reply (`format` 32, server byte order
/// = ours, as the client announced at connect), `None` for a missing/empty/other
/// format property or the value 0 (no window / no pid).
pub fn card32(format: u8, value: &[u8]) -> Option<u32> {
    if format != 32 {
        return None;
    }
    let b: [u8; 4] = value.get(..4)?.try_into().ok()?;
    Some(u32::from_ne_bytes(b)).filter(|&v| v != 0)
}

/// `/proc/<pid>/exe`, the link to the process's executable.
pub fn proc_exe(pid: u32) -> String {
    format!("/proc/{pid}/exe")
}

/// The executable path from a `/proc/<pid>/exe` link target. The kernel appends
/// ` (deleted)` when the binary was replaced (e.g. by a package update while the
/// app runs); the name is still the right one, so the suffix is dropped. Anything
/// that is not an absolute path (e.g. `anon_inode:…`) is rejected.
pub fn exe_from_link(target: &str) -> Option<String> {
    let path = target.strip_suffix(" (deleted)").unwrap_or(target);
    (path.starts_with('/') && path.len() > 1).then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_wins_over_display() {
        assert_eq!(
            session(Some("wayland-0"), None, Some(":0")),
            Session::Wayland
        );
        assert_eq!(session(None, Some("wayland"), Some(":0")), Session::Wayland);
        assert_eq!(session(Some(" "), Some("x11"), Some(":0")), Session::X11);
        assert_eq!(session(None, Some("tty"), None), Session::Headless);
        assert_eq!(session(None, None, Some("")), Session::Headless);
    }

    #[test]
    fn card32_reads_first_item() {
        let v = 0x0340_0007u32.to_ne_bytes();
        assert_eq!(card32(32, &v), Some(0x0340_0007));
        let mut two = v.to_vec();
        two.extend_from_slice(&5u32.to_ne_bytes());
        assert_eq!(card32(32, &two), Some(0x0340_0007));
    }

    #[test]
    fn card32_rejects_missing_zero_and_wrong_format() {
        assert_eq!(card32(0, &[]), None); // property not set
        assert_eq!(card32(32, &[1, 2]), None);
        assert_eq!(card32(32, &0u32.to_ne_bytes()), None); // no active window
        assert_eq!(card32(8, b"abcd"), None);
    }

    #[test]
    fn proc_path() {
        assert_eq!(proc_exe(4242), "/proc/4242/exe");
    }

    #[test]
    fn link_target_to_exe() {
        assert_eq!(
            exe_from_link("/usr/lib/firefox/firefox").as_deref(),
            Some("/usr/lib/firefox/firefox")
        );
        assert_eq!(
            exe_from_link("/usr/bin/code (deleted)").as_deref(),
            Some("/usr/bin/code")
        );
        assert_eq!(exe_from_link("anon_inode:[eventfd]"), None);
        assert_eq!(exe_from_link("/"), None);
        assert_eq!(exe_from_link(""), None);
    }

    /// The contract with the rules: Linux paths match by file name like Windows ones.
    #[test]
    fn linux_paths_match_rules_by_file_name() {
        use opensynapse_core::app_profiles::exe_matches;
        let exe = exe_from_link("/usr/lib/firefox/firefox").unwrap();
        assert!(exe_matches("firefox", &exe));
        assert!(exe_matches("Firefox", &exe));
        assert!(exe_matches("/usr/lib/firefox/firefox", &exe));
        assert!(!exe_matches("firefox.exe", &exe));
    }
}
