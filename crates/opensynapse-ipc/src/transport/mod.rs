//! Per-user local endpoint. Windows: named pipe
//! `\\.\pipe\opensynapse-agent-<user SID>`, reachable only by the same user;
//! Unix: a 0600 socket in `$XDG_RUNTIME_DIR`, on macOS in
//! `~/Library/Application Support/OpenSynapse` ([`socket_path`]; untested on
//! hardware).
//!
//! API (same on every platform):
//! - [`default_pipe_name`]: the current user's endpoint name.
//! - [`Listener::bind`] / [`Listener::accept`] / [`Listener::waker`]: server side.
//!   `bind` fails with `AddrInUse` when another agent already serves the name.
//! - [`Waker::wake`]: unblocks a pending `accept` from another thread (the next
//!   accepted connection is the waker's own; the caller checks its stop flag).
//! - [`connect`]: client side; `NotFound` when no agent runs, `PermissionDenied`
//!   when the server runs as another user.
//! - [`Conn`]: one blocking byte stream (`Read + Write + Send`).

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Conn, Listener, Waker, connect, default_pipe_name};

#[cfg(any(unix, test))]
pub mod socket_path;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{Conn, Listener, Waker, connect, default_pipe_name};
