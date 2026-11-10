//! Windows transport: a per-user named pipe (ADR-0005).
//!
//! - Name `\\.\pipe\opensynapse-agent-<user SID>`.
//! - Every instance carries a protected DACL with one allow entry for the current
//!   user (`D:P(A;;GA;;;<SID>)`) and `PIPE_REJECT_REMOTE_CLIENTS`.
//! - The first instance uses `FILE_FLAG_FIRST_PIPE_INSTANCE`: if anyone already
//!   owns the name, `bind` fails with `AddrInUse` (single agent per user).
//! - The pipe namespace is global, so another user could squat the name before
//!   the agent starts; the client therefore checks that the server process runs as
//!   the same user (`GetNamedPipeServerProcessId` → token user → `EqualSid`), and
//!   it connects with `SECURITY_IDENTIFICATION` so a server cannot impersonate it.
//! - Blocking byte-mode I/O, no overlapped I/O: a connection has one role.

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::io::{self, Read, Write};
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Pipe buffer size per direction; frames larger than this just take more writes.
const BUFFER: u32 = 4096;
/// How long a client waits once for a free instance (`ERROR_PIPE_BUSY`).
const BUSY_WAIT_MS: u32 = 2000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn is_os_error(e: &io::Error, code: u32) -> bool {
    e.raw_os_error() == Some(code as i32)
}

/// An owned kernel handle, closed once on drop.
struct Handle(HANDLE);

// SAFETY: kernel handles may be used and closed from any thread.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: the handle came from a successful Create*/Open* call and is
            // owned by this value only, so it is closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A `TOKEN_USER` buffer; [`Self::sid`] points into it.
struct UserSid {
    /// `u64` elements keep the buffer pointer-aligned for `TOKEN_USER`.
    buf: Vec<u64>,
}

impl UserSid {
    fn of_token(token: HANDLE) -> io::Result<Self> {
        let mut len = 0u32;
        // SAFETY: size query: null buffer with length 0; `len` is a valid out-pointer.
        // The call fails with ERROR_INSUFFICIENT_BUFFER and sets `len`.
        unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len) };
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(size_of::<u64>())];
        // SAFETY: `buf` is writable for at least `len` bytes and 8-byte aligned.
        let ok = unsafe {
            GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { buf })
    }

    fn sid(&self) -> PSID {
        // SAFETY: `buf` holds a TOKEN_USER written by GetTokenInformation and is
        // suitably aligned; the SID it points to lives inside `buf`.
        unsafe { (*self.buf.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

/// The user a process runs as.
fn process_user(process: HANDLE) -> io::Result<UserSid> {
    let mut token: HANDLE = null_mut();
    // SAFETY: `process` is a valid process handle; `token` is a valid out-pointer.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    UserSid::of_token(token.0)
}

fn current_user() -> io::Result<UserSid> {
    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no closing.
    process_user(unsafe { GetCurrentProcess() })
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut p: *mut u16 = null_mut();
    // SAFETY: `sid` points to a valid SID; on success `p` is a NUL-terminated
    // string allocated with LocalAlloc, freed below.
    if unsafe { ConvertSidToStringSidW(sid, &mut p) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `p` is NUL-terminated (see above); we read up to the terminator.
    let s = unsafe {
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    };
    // SAFETY: `p` came from ConvertSidToStringSidW and is freed exactly once.
    unsafe { LocalFree(p.cast()) };
    Ok(s)
}

/// The current user's pipe name.
pub fn default_pipe_name() -> io::Result<String> {
    let sid = sid_string(current_user()?.sid())?;
    Ok(format!(r"\\.\pipe\opensynapse-agent-{sid}"))
}

/// A self-relative security descriptor from LocalAlloc, freed on drop.
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is immutable heap memory owned by this value.
unsafe impl Send for SecurityDescriptor {}

impl SecurityDescriptor {
    /// Protected DACL with one entry: full access for the current user.
    fn current_user_only() -> io::Result<Self> {
        let sid = sid_string(current_user()?.sid())?;
        let sddl = wide(&format!("D:P(A;;GA;;;{sid})"));
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `sddl` is NUL-terminated and outlives the call; `sd` is a valid
        // out-pointer; the size out-pointer may be null.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(sd))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW
        // (LocalAlloc) and freed exactly once.
        unsafe { LocalFree(self.0) };
    }
}

/// Server side: the first instance makes this process the only agent.
pub struct Listener {
    name: Arc<Vec<u16>>,
    sd: SecurityDescriptor,
    /// The instance waiting for the next client. Always present, so clients see
    /// `ERROR_PIPE_BUSY` (and wait) rather than `ERROR_FILE_NOT_FOUND`.
    next: Handle,
}

impl Listener {
    /// `AddrInUse` when another agent already serves `name`.
    pub fn bind(name: &str) -> io::Result<Self> {
        let name = Arc::new(wide(name));
        let sd = SecurityDescriptor::current_user_only()?;
        let next = create_instance(&name, &sd, true)?;
        Ok(Self { name, sd, next })
    }

    /// Block until a client connects. The next instance exists before the
    /// connected one is handed out.
    pub fn accept(&mut self) -> io::Result<Conn> {
        loop {
            // SAFETY: `next` is a valid pipe instance handle; no overlapped I/O.
            if unsafe { ConnectNamedPipe(self.next.0, null_mut()) } == 0 {
                let e = io::Error::last_os_error();
                if is_os_error(&e, ERROR_NO_DATA) {
                    // A client connected and already closed: reset and wait again.
                    // SAFETY: valid pipe instance handle.
                    unsafe { DisconnectNamedPipe(self.next.0) };
                    continue;
                }
                // ERROR_PIPE_CONNECTED: a client connected before ConnectNamedPipe.
                if !is_os_error(&e, ERROR_PIPE_CONNECTED) {
                    return Err(e);
                }
            }
            break;
        }
        let fresh = match create_instance(&self.name, &self.sd, false) {
            Ok(h) => h,
            Err(e) => {
                // Drop this client but keep the instance for the next accept.
                // SAFETY: valid pipe instance handle.
                unsafe { DisconnectNamedPipe(self.next.0) };
                return Err(e);
            }
        };
        let connected = std::mem::replace(&mut self.next, fresh);
        Ok(Conn { h: connected })
    }

    /// Handle that unblocks a pending [`Self::accept`] from another thread.
    pub fn waker(&self) -> Waker {
        Waker {
            name: Arc::clone(&self.name),
        }
    }
}

fn create_instance(name: &[u16], sd: &SecurityDescriptor, first: bool) -> io::Result<Handle> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let open_mode = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // SAFETY: `name` is NUL-terminated; `sa` and the descriptor it points to
    // outlive the call.
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            0,
            &sa,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        let e = io::Error::last_os_error();
        if first && (is_os_error(&e, ERROR_ACCESS_DENIED) || is_os_error(&e, ERROR_PIPE_BUSY)) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another process already serves this pipe name",
            ));
        }
        return Err(e);
    }
    Ok(Handle(h))
}

/// Unblocks [`Listener::accept`] by connecting to the pipe and hanging up.
#[derive(Clone, Debug)]
pub struct Waker {
    name: Arc<Vec<u16>>,
}

impl Waker {
    pub fn wake(&self) {
        // Errors are irrelevant: if nobody listens, nothing needs waking.
        let _ = open_pipe(&self.name);
    }
}

/// `CreateFileW` on the pipe; on `ERROR_PIPE_BUSY` wait once for a free instance.
fn open_pipe(name: &[u16]) -> io::Result<Handle> {
    let mut waited = false;
    loop {
        // SAFETY: `name` is NUL-terminated and outlives the call; no security
        // attributes or template handle.
        let h = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            return Ok(Handle(h));
        }
        let e = io::Error::last_os_error();
        if !is_os_error(&e, ERROR_PIPE_BUSY) || waited {
            return Err(e);
        }
        waited = true;
        // SAFETY: `name` is NUL-terminated and outlives the call.
        if unsafe { WaitNamedPipeW(name.as_ptr(), BUSY_WAIT_MS) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
}

/// Client side. Verifies that the server runs as the same user
/// (`PermissionDenied` otherwise); `NotFound` when no agent runs.
pub fn connect(name: &str) -> io::Result<Conn> {
    let h = open_pipe(&wide(name)).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no OpenSynapse agent is listening on {name}"),
            )
        } else {
            e
        }
    })?;
    verify_server(&h)?;
    Ok(Conn { h })
}

fn denied(what: &str, e: io::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("cannot verify the pipe server ({what}): {e}"),
    )
}

/// The server process must run as the current user.
fn verify_server(pipe: &Handle) -> io::Result<()> {
    let mut pid = 0u32;
    // SAFETY: `pipe` is a valid client pipe handle; `pid` is a valid out-pointer.
    if unsafe { GetNamedPipeServerProcessId(pipe.0, &mut pid) } == 0 {
        return Err(denied("server process id", io::Error::last_os_error()));
    }
    // SAFETY: plain call; a null result is checked below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(denied("open process", io::Error::last_os_error()));
    }
    let process = Handle(process);
    let server = process_user(process.0).map_err(|e| denied("server token", e))?;
    let me = current_user()?;
    // SAFETY: both SIDs are valid and live inside their buffers for the call.
    if unsafe { EqualSid(me.sid(), server.sid()) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the pipe server runs as another user",
        ));
    }
    Ok(())
}

/// One connected byte stream.
pub struct Conn {
    h: Handle,
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let mut n = 0u32;
        // SAFETY: `buf` is writable for `len` bytes; synchronous (no overlapped).
        let ok = unsafe { ReadFile(self.h.0, buf.as_mut_ptr(), len, &mut n, null_mut()) };
        if ok == 0 {
            let e = io::Error::last_os_error();
            // The other end closed: end of stream.
            if is_os_error(&e, ERROR_BROKEN_PIPE) || is_os_error(&e, ERROR_PIPE_NOT_CONNECTED) {
                return Ok(0);
            }
            return Err(e);
        }
        Ok(n as usize)
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let mut n = 0u32;
        // SAFETY: `buf` is readable for `len` bytes; synchronous (no overlapped).
        let ok = unsafe { WriteFile(self.h.0, buf.as_ptr(), len, &mut n, null_mut()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    /// No-op: `WriteFile` hands the bytes to the pipe; `FlushFileBuffers` would
    /// block until the peer has read them.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
