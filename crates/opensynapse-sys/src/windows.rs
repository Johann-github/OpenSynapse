//! Windows side: Run-key registry value, token elevation, parent console.

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
    HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
    WIN32_ERROR,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
    STD_OUTPUT_HANDLE, SetStdHandle,
};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
    WaitForSingleObject,
};

use crate::autostart::VALUE_NAME;

/// Make this process's stdin/stdout/stderr handles non-inheritable.
///
/// `CreateProcess` (used by `std::process::Command`) inherits *every* inheritable
/// handle, not only the child's stdio. A detached agent started from a shell pipe
/// (`opensynapse agent start | tail`, `install | grep` in Git Bash) would otherwise
/// keep the pipe's write end open for its whole lifetime, so the reader never sees
/// EOF and hangs (observed 2026-11-11). std duplicates handles it passes to a
/// child as inheritable copies, so later `Stdio::inherit` spawns still work.
/// Best effort: a missing or invalid handle is skipped.
pub fn stop_std_handle_inheritance() {
    use windows_sys::Win32::System::Console::STD_INPUT_HANDLE;
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle has no preconditions; SetHandleInformation only
        // changes a flag on a handle this process owns and fails harmlessly on
        // a null or invalid one.
        unsafe {
            let h = GetStdHandle(which);
            if !h.is_null() && h != INVALID_HANDLE_VALUE {
                SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// An open process handle (`SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION`).
pub(crate) struct Process(HANDLE);

impl Process {
    pub(crate) fn open(pid: u32) -> io::Result<Self> {
        // SAFETY: no pointer arguments; a null result is checked.
        let h = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(h))
    }

    pub(crate) fn image_path(&self) -> io::Result<std::path::PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        let mut buf = vec![0u16; 32 * 1024];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` is writable for `len` u16s; the handle is open.
        let ok = unsafe {
            QueryFullProcessImageNameW(self.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(std::ffi::OsString::from_wide(&buf[..len as usize]).into())
    }

    pub(crate) fn wait_exit(&self, timeout: std::time::Duration) -> io::Result<bool> {
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        // SAFETY: the handle is open with SYNCHRONIZE access.
        match unsafe { WaitForSingleObject(self.0, ms) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: opened in `open`, closed exactly once here.
        unsafe { CloseHandle(self.0) };
    }
}

/// The per-user Run key, relative to `HKEY_CURRENT_USER`.
pub(crate) const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn check(code: WIN32_ERROR) -> io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

pub(crate) fn run_get(key: &str) -> io::Result<Option<String>> {
    let (key, name) = (wide(key), wide(VALUE_NAME));
    let mut len = 0u32;
    // SAFETY: size query: key/name are NUL-terminated, data pointer null, `len` valid.
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut len,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(code)?;
    let mut buf = vec![0u16; (len as usize).div_ceil(2)];
    // SAFETY: `buf` is writable for `len` bytes; RegGetValueW NUL-terminates REG_SZ.
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast::<c_void>(),
            &mut len,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(code)?;
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(Some(String::from_utf16_lossy(&buf[..end])))
}

pub(crate) fn run_set(key: &str, command: &str) -> io::Result<()> {
    let (key, name, data) = (wide(key), wide(VALUE_NAME), wide(command));
    let bytes = u32::try_from(data.len() * 2).map_err(|_| io::Error::other("command too long"))?;
    // SAFETY: all strings are NUL-terminated; `data` is readable for `bytes` bytes
    // (including the terminator, as REG_SZ requires). Creates the key if missing.
    check(unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            data.as_ptr().cast::<c_void>(),
            bytes,
        )
    })
}

pub(crate) fn run_delete(key: &str) -> io::Result<bool> {
    let (key, name) = (wide(key), wide(VALUE_NAME));
    // SAFETY: both strings are NUL-terminated.
    let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(false);
    }
    check(code).map(|()| true)
}

pub(crate) fn is_elevated() -> io::Result<bool> {
    let mut token: HANDLE = null_mut();
    // SAFETY: GetCurrentProcess is a pseudo handle; `token` is a valid out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut len = 0u32;
    // SAFETY: `e` is a writable TOKEN_ELEVATION of the size passed; `token` is open.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut e as *mut TOKEN_ELEVATION).cast::<c_void>(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    let err = io::Error::last_os_error();
    // SAFETY: `token` was opened above and is closed exactly once.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return Err(err);
    }
    Ok(e.TokenIsElevated != 0)
}

fn std_handle_set(which: STD_HANDLE) -> bool {
    // SAFETY: GetStdHandle has no preconditions.
    let h = unsafe { GetStdHandle(which) };
    !h.is_null() && h != INVALID_HANDLE_VALUE
}

pub(crate) fn attach_parent_console() -> bool {
    let out_set = std_handle_set(STD_OUTPUT_HANDLE);
    let err_set = std_handle_set(STD_ERROR_HANDLE);
    if out_set && err_set {
        return false; // the parent redirected both; keep them
    }
    // SAFETY: no preconditions; fails harmlessly if the parent has no console.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return false;
    }
    let conout = wide("CONOUT$");
    // SAFETY: `conout` is NUL-terminated. The handle becomes the standard
    // output/error handle for the process's life, so it is never closed.
    let h = unsafe {
        CreateFileW(
            conout.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return false;
    }
    // SAFETY: `h` is a valid console output handle.
    unsafe {
        if !out_set {
            SetStdHandle(STD_OUTPUT_HANDLE, h);
        }
        if !err_set {
            SetStdHandle(STD_ERROR_HANDLE, h);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Registry::RegDeleteKeyW;

    /// Round trip on a scratch key under HKCU, never the real Run key.
    #[test]
    fn registry_round_trip() {
        let key = format!(r"Software\OpenSynapse-test-{}", std::process::id());
        assert_eq!(run_get(&key).unwrap(), None);
        assert!(!run_delete(&key).unwrap());
        run_set(&key, r#""C:\x y\opensynapse-agent.exe""#).unwrap();
        assert_eq!(
            run_get(&key).unwrap().as_deref(),
            Some(r#""C:\x y\opensynapse-agent.exe""#)
        );
        run_set(&key, "second").unwrap();
        assert_eq!(run_get(&key).unwrap().as_deref(), Some("second"));
        assert!(run_delete(&key).unwrap());
        assert_eq!(run_get(&key).unwrap(), None);
        let k = wide(&key);
        // SAFETY: `k` is NUL-terminated; removes the now empty scratch key.
        unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, k.as_ptr()) };
    }

    #[test]
    fn elevation_query_works() {
        is_elevated().unwrap();
    }

    /// Wait on a short-lived child by pid: times out while it runs, then sees the exit.
    #[test]
    fn process_wait_and_image_path() {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 2 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        let p = Process::open(child.id()).unwrap();
        let img = p.image_path().unwrap();
        assert!(
            img.to_string_lossy().to_lowercase().ends_with("cmd.exe"),
            "{img:?}"
        );
        assert!(!p.wait_exit(std::time::Duration::ZERO).unwrap());
        assert!(p.wait_exit(std::time::Duration::from_secs(10)).unwrap());
        child.wait().unwrap();
    }
}
