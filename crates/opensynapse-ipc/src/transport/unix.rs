//! Unix transport: a stream socket with mode 0600 at
//! [`super::socket_path::socket_path`] (`$XDG_RUNTIME_DIR/opensynapse-agent.sock`;
//! macOS `~/Library/Application Support/OpenSynapse/opensynapse-agent.sock`).
//! Both folders are private to the user (0700), so only the same user can reach
//! the socket. Untested on hardware (ADR-0012).

use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;

use super::socket_path::{Layout, socket_path};

/// The current user's socket path.
pub fn default_pipe_name() -> io::Result<String> {
    let var = |k: &str| -> io::Result<Option<String>> {
        std::env::var_os(k)
            .map(|v| {
                v.into_string().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("{k} is not UTF-8"))
                })
            })
            .transpose()
    };
    match Layout::CURRENT {
        Layout::Xdg => socket_path(Layout::Xdg, var("XDG_RUNTIME_DIR")?.as_deref(), None),
        Layout::MacOs => socket_path(Layout::MacOs, None, var("HOME")?.as_deref()),
    }
}

/// Server side: the bound socket makes this process the only agent.
pub struct Listener {
    inner: UnixListener,
    path: Arc<PathBuf>,
}

impl Listener {
    /// `AddrInUse` when another agent already serves `name`. A stale socket file
    /// (nobody accepting) is replaced.
    pub fn bind(name: &str) -> io::Result<Self> {
        let path = PathBuf::from(name);
        // macOS: `~/Library/Application Support/OpenSynapse` may not exist yet.
        // Created private (0700); an existing folder is left as it is.
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
            && !dir.exists()
        {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another process already serves this socket",
                ));
            }
            std::fs::remove_file(&path)?;
        }
        let inner = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            inner,
            path: Arc::new(path),
        })
    }

    /// Block until a client connects.
    pub fn accept(&mut self) -> io::Result<Conn> {
        let (s, _) = self.inner.accept()?;
        Ok(Conn { s })
    }

    /// Handle that unblocks a pending [`Self::accept`] from another thread.
    pub fn waker(&self) -> Waker {
        Waker {
            path: Arc::clone(&self.path),
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&*self.path);
    }
}

/// Unblocks [`Listener::accept`] by connecting and hanging up.
#[derive(Clone, Debug)]
pub struct Waker {
    path: Arc<PathBuf>,
}

impl Waker {
    pub fn wake(&self) {
        let _ = UnixStream::connect(&*self.path);
    }
}

/// Client side; `NotFound` when no agent runs.
pub fn connect(name: &str) -> io::Result<Conn> {
    let s = UnixStream::connect(name).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => io::Error::new(
            io::ErrorKind::NotFound,
            format!("no OpenSynapse agent is listening on {name}"),
        ),
        _ => e,
    })?;
    Ok(Conn { s })
}

/// One connected byte stream.
pub struct Conn {
    s: UnixStream,
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.s.read(buf)
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.s.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.s.flush()
    }
}
