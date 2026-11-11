//! Windows side: Run-key registry value, token elevation, parent console.

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, WIN32_ERROR,
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
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::autostart::VALUE_NAME;

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
}
