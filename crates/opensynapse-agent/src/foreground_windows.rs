//! Windows foreground watch: `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` out of
//! context, delivered to a dedicated thread that blocks in `GetMessageW`. The
//! callback resolves window → process id → `QueryFullProcessImageNameW`.
//! Shutdown: `PostThreadMessageW(WM_QUIT)`, then the thread unhooks itself.

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::cell::RefCell;
use std::io;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::sync::mpsc::sync_channel;
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, GetLastError, HWND};
use windows_sys::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetMessageW,
    GetWindowThreadProcessId, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};

use super::{Dedup, OnForeground};

/// The hook thread's callback and de-duplication. The WinEvent callback has no
/// user-data pointer, but out-of-context hooks are delivered on the thread that
/// installed them, so a thread-local is enough.
struct State {
    f: OnForeground,
    dedup: Dedup,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The running hook thread.
pub struct Watch {
    thread_id: u32,
    handle: Option<JoinHandle<()>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: plain message post to a thread id; the thread created its queue
        // before reporting its id (see `run`), so the post cannot be lost.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn watch(f: OnForeground) -> io::Result<Watch> {
    let (ready_tx, ready_rx) = sync_channel::<io::Result<u32>>(1);
    let handle = std::thread::Builder::new()
        .name("foreground".into())
        .stack_size(256 * 1024)
        .spawn(move || run(f, |r| drop(ready_tx.send(r))))?;
    match ready_rx.recv() {
        Ok(Ok(thread_id)) => Ok(Watch {
            thread_id,
            handle: Some(handle),
        }),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => {
            let _ = handle.join();
            Err(io::Error::other(
                "the foreground thread stopped during start",
            ))
        }
    }
}

/// Thread body: create the queue, hook, report, pump until `WM_QUIT`, unhook.
fn run(f: OnForeground, ready: impl FnOnce(io::Result<u32>)) {
    // SAFETY: MSG is plain data; all-zero is a valid value.
    let mut msg: MSG = unsafe { zeroed() };
    // SAFETY: `msg` is a valid out pointer. Creates this thread's message queue so
    // that `PostThreadMessageW(WM_QUIT)` works from now on; removes nothing.
    unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE) };

    STATE.with_borrow_mut(|s| {
        *s = Some(State {
            f,
            dedup: Dedup::default(),
        })
    });
    // SAFETY: `on_event` matches WINEVENTPROC and lives for the whole program; no
    // module handle is needed for an out-of-context hook.
    let hook: HWINEVENTHOOK = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            null_mut(),
            Some(on_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if hook.is_null() {
        ready(Err(io::Error::last_os_error()));
        STATE.with_borrow_mut(|s| *s = None);
        return;
    }
    // SAFETY: no arguments.
    ready(Ok(unsafe { GetCurrentThreadId() }));

    // The application in front right now.
    // SAFETY: no arguments; a null result is handled in `report`.
    report(unsafe { GetForegroundWindow() });

    loop {
        // SAFETY: `msg` is a valid out pointer. Blocks in the kernel until a message
        // or a hook event arrives (hook callbacks run inside this call).
        let r = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if r == 0 || r == -1 {
            break; // WM_QUIT, or an error that would repeat forever
        }
        // SAFETY: `msg` was filled by GetMessageW.
        unsafe { DispatchMessageW(&msg) };
    }
    // SAFETY: `hook` came from SetWinEventHook on this thread and is unhooked once.
    unsafe { UnhookWinEvent(hook) };
    STATE.with_borrow_mut(|s| *s = None);
}

/// WinEvent callback (runs on the hook thread inside `GetMessageW`).
unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND {
        report(hwnd);
    }
}

/// Resolve `hwnd` to its executable and call back if it changed.
fn report(hwnd: HWND) {
    let Some(exe) = exe_of(hwnd) else {
        return;
    };
    STATE.with_borrow_mut(|s| {
        if let Some(s) = s
            && s.dedup.changed(&exe)
        {
            (s.f)(exe);
        }
    });
}

/// Full image path of the process that owns `hwnd`; `None` for no window or a
/// process this user may not query.
fn exe_of(hwnd: HWND) -> Option<String> {
    if hwnd.is_null() {
        return None;
    }
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out pointer; a stale `hwnd` just yields 0.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == 0 {
        return None;
    }
    // SAFETY: plain call; the handle is closed below.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        tracing::trace!(pid, "cannot open the foreground process");
        return None;
    }
    let mut out = None;
    for cap in [1024usize, 32 * 1024] {
        let mut buf = vec![0u16; cap];
        let mut len = cap as u32;
        // SAFETY: `buf` holds `len` u16s; `len` gets the written length.
        let ok = unsafe {
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
        };
        if ok != 0 {
            out = Some(String::from_utf16_lossy(&buf[..len as usize]));
            break;
        }
        // SAFETY: no arguments.
        if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            break;
        }
    }
    // SAFETY: `h` is an open process handle, closed once.
    unsafe { CloseHandle(h) };
    out
}
