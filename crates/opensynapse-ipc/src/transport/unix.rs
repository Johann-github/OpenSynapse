//! Unix transport (compile-only for now): a stream socket at
//! `$XDG_RUNTIME_DIR/opensynapse-agent.sock` with mode 0600. `$XDG_RUNTIME_DIR`
//! is per user and 0700, so only the same user can reach the socket.

use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;

/// The current user's socket path.
pub fn default_pipe_name() -> io::Result<String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    let path = PathBuf::from(dir).join("opensynapse-agent.sock");
    path.into_os_string()
        .into_string()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "socket path is not UTF-8"))
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
