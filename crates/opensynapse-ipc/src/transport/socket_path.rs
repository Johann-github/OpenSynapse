//! Where the Unix agent socket lives (pure, unit-tested on every OS;
//! `vault/Decisions/ADR-0012 macOS agent.md`).
//!
//! - Linux and other Unix: `$XDG_RUNTIME_DIR/opensynapse-agent.sock`. The
//!   runtime dir is per user and 0700; unset or blank is an error (unchanged
//!   since stage 2, except that a blank value no longer yields a relative path).
//! - macOS: `$HOME/Library/Application Support/OpenSynapse/opensynapse-agent.sock`.
//!   macOS sets no `XDG_RUNTIME_DIR`, and a value a user exports in a shell would
//!   not reach the launchd-started agent, so it is ignored there. `$TMPDIR` was
//!   rejected: its per-user `/var/folders/…/T` is cleaned by the system, which
//!   could delete a live socket. `~/Library` is 0700; the server creates the
//!   `OpenSynapse` folder with mode 0700 when it is missing.
//!
//! `sun_path` holds 104 bytes on macOS and 108 on Linux, NUL included; a longer
//! path is refused with a clear error instead of a confusing bind failure.

use std::io;

/// Socket file name (both layouts).
pub const SOCKET_FILE: &str = "opensynapse-agent.sock";
/// Folders under `$HOME` that hold the socket on macOS.
pub const MACOS_DIR: [&str; 3] = ["Library", "Application Support", "OpenSynapse"];
/// Longest socket path in bytes (without the NUL) on macOS (`sun_path[104]`).
pub const MAX_LEN_MACOS: usize = 103;
/// Longest socket path in bytes (without the NUL) on Linux (`sun_path[108]`).
pub const MAX_LEN_LINUX: usize = 107;

/// Which layout to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `$XDG_RUNTIME_DIR` (Linux and every Unix except macOS).
    Xdg,
    /// `~/Library/Application Support/OpenSynapse` (macOS).
    MacOs,
}

impl Layout {
    /// The layout of the OS this was compiled for.
    pub const CURRENT: Layout = if cfg!(target_os = "macos") {
        Layout::MacOs
    } else {
        Layout::Xdg
    };

    fn max_len(self) -> usize {
        match self {
            Layout::Xdg => MAX_LEN_LINUX,
            Layout::MacOs => MAX_LEN_MACOS,
        }
    }
}

/// The socket path for the given environment values (`XDG_RUNTIME_DIR` is used
/// by [`Layout::Xdg`] only, `HOME` by [`Layout::MacOs`] only).
pub fn socket_path(
    layout: Layout,
    xdg_runtime_dir: Option<&str>,
    home: Option<&str>,
) -> io::Result<String> {
    fn nonblank(v: Option<&str>) -> Option<&str> {
        v.filter(|s| !s.trim().is_empty())
    }
    let path = match layout {
        Layout::Xdg => {
            let dir = nonblank(xdg_runtime_dir).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set")
            })?;
            join(dir, &[SOCKET_FILE])
        }
        Layout::MacOs => {
            let home = nonblank(home)
                .filter(|h| h.starts_with('/'))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "HOME is not set to an absolute path",
                    )
                })?;
            let mut parts = MACOS_DIR.to_vec();
            parts.push(SOCKET_FILE);
            join(home, &parts)
        }
    };
    if path.len() > layout.max_len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "socket path {path} is {} bytes long; at most {} fit in a Unix socket address",
                path.len(),
                layout.max_len()
            ),
        ));
    }
    Ok(path)
}

/// `base/part/part…` with exactly one `/` between parts (Unix separators on
/// every OS, so tests on Windows see the same string).
fn join(base: &str, parts: &[&str]) -> String {
    let mut out = base.trim_end_matches('/').to_owned();
    if out.is_empty() {
        out.push('/');
    }
    for p in parts {
        if !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_layout_uses_runtime_dir_only() {
        assert_eq!(
            socket_path(Layout::Xdg, Some("/run/user/1000"), Some("/home/u")).unwrap(),
            "/run/user/1000/opensynapse-agent.sock"
        );
        assert_eq!(
            socket_path(Layout::Xdg, Some("/run/user/1000/"), None).unwrap(),
            "/run/user/1000/opensynapse-agent.sock"
        );
        for missing in [None, Some(""), Some("  ")] {
            let e = socket_path(Layout::Xdg, missing, Some("/home/u")).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::NotFound);
            assert!(e.to_string().contains("XDG_RUNTIME_DIR"), "{e}");
        }
    }

    #[test]
    fn macos_layout_uses_home_only() {
        let want = "/Users/u/Library/Application Support/OpenSynapse/opensynapse-agent.sock";
        assert_eq!(
            socket_path(Layout::MacOs, None, Some("/Users/u")).unwrap(),
            want
        );
        assert_eq!(
            socket_path(Layout::MacOs, None, Some("/Users/u/")).unwrap(),
            want
        );
        // An exported XDG_RUNTIME_DIR is ignored (launchd would not pass it on).
        assert_eq!(
            socket_path(Layout::MacOs, Some("/tmp/xdg"), Some("/Users/u")).unwrap(),
            want
        );
        for bad in [None, Some(""), Some("relative/home")] {
            let e = socket_path(Layout::MacOs, None, bad).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::NotFound, "{bad:?}");
        }
        assert_eq!(
            socket_path(Layout::MacOs, None, Some("/")).unwrap(),
            "/Library/Application Support/OpenSynapse/opensynapse-agent.sock"
        );
    }

    #[test]
    fn too_long_paths_are_refused() {
        // Fixed part on macOS: "/Library/Application Support/OpenSynapse/" + file.
        let fixed = socket_path(Layout::MacOs, None, Some("/")).unwrap().len();
        let fits = format!("/{}", "a".repeat(MAX_LEN_MACOS - fixed - 1));
        let p = socket_path(Layout::MacOs, None, Some(&fits)).unwrap();
        assert_eq!(p.len(), MAX_LEN_MACOS);
        let long = format!("{fits}b");
        let e = socket_path(Layout::MacOs, None, Some(&long)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert!(
            e.to_string().contains("104") || e.to_string().contains("103"),
            "{e}"
        );

        let dir = format!("/{}", "r".repeat(MAX_LEN_LINUX - SOCKET_FILE.len() - 2));
        assert_eq!(
            socket_path(Layout::Xdg, Some(&dir), None).unwrap().len(),
            MAX_LEN_LINUX
        );
        assert!(socket_path(Layout::Xdg, Some(&format!("{dir}x")), None).is_err());
    }

    #[test]
    fn current_layout_matches_target() {
        assert_eq!(Layout::CURRENT == Layout::MacOs, cfg!(target_os = "macos"));
    }
}
