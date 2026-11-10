//! Per-user local endpoint. Windows: named pipe
//! `\\.\pipe\opensynapse-agent-<user SID>`, reachable only by the same user;
//! Unix: a 0600 socket in `$XDG_RUNTIME_DIR` (later).

use std::io::{self, Read, Write};

/// The current user's pipe name.
pub fn default_pipe_name() -> io::Result<String> {
    todo!("ipc transport: default_pipe_name")
}

/// Server side: the first instance makes this process the only agent.
pub struct Listener {
    _priv: (),
}

impl Listener {
    /// `AddrInUse` when another agent already serves `name`.
    pub fn bind(name: &str) -> io::Result<Self> {
        let _ = name;
        todo!("ipc transport: Listener::bind")
    }

    /// Block until a client connects.
    pub fn accept(&mut self) -> io::Result<Conn> {
        todo!("ipc transport: Listener::accept")
    }

    /// Handle that unblocks a pending [`Self::accept`] from another thread.
    pub fn waker(&self) -> Waker {
        todo!("ipc transport: Listener::waker")
    }
}

/// Unblocks [`Listener::accept`].
#[derive(Clone, Debug)]
pub struct Waker {
    _priv: (),
}

impl Waker {
    pub fn wake(&self) {
        todo!("ipc transport: Waker::wake")
    }
}

/// One connected byte stream.
pub struct Conn {
    _priv: (),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let _ = buf;
        todo!("ipc transport: Conn::read")
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = buf;
        todo!("ipc transport: Conn::write")
    }

    fn flush(&mut self) -> io::Result<()> {
        todo!("ipc transport: Conn::flush")
    }
}

/// Client side. Verifies that the server runs as the same user
/// (`PermissionDenied` otherwise); `NotFound` when no agent runs.
pub fn connect(name: &str) -> io::Result<Conn> {
    let _ = name;
    todo!("ipc transport: connect")
}
