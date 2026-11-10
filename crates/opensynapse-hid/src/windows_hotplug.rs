//! Windows hotplug: `CM_Register_Notification` for the HID device-interface class.
//! No window, no message loop, no polling; the system calls back on its own
//! thread pool. See `hotplug.rs` for the public API.

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::ptr::{addr_of, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL,
    CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
    CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification, CM_Unregister_Notification,
    CR_SUCCESS, HCMNOTIFICATION,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::HidD_GetHidGuid;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::core::GUID;

use crate::HidError;
use crate::hotplug::{Callback, HotplugEvent, is_vendor_path};

/// What the system hands back to [`on_notify`].
struct Context {
    vid: u16,
    f: Callback,
}

/// A live registration. Drop unregisters, then frees the context.
pub struct Registration {
    handle: HCMNOTIFICATION,
    context: *mut Context,
}

// SAFETY: the handle is used once more (in Drop), which may run on any thread; the
// context is Send + Sync (its closure is) and is only freed after unregistering.
unsafe impl Send for Registration {}

impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: `handle` came from a successful CM_Register_Notification and is
        // unregistered once. The call waits for running callbacks to finish, so no
        // callback can touch `context` afterwards. Never called from the callback.
        unsafe { CM_Unregister_Notification(self.handle) };
        // SAFETY: `context` came from Box::into_raw in `register`, nothing uses it any
        // more (see above), and it is freed exactly once.
        drop(unsafe { Box::from_raw(self.context) });
    }
}

/// Register `f` for HID interface arrivals/removals of vendor `vid`.
pub fn register(vid: u16, f: Callback) -> Result<Registration, HidError> {
    let mut guid = GUID::from_u128(0);
    // SAFETY: writes one GUID.
    unsafe { HidD_GetHidGuid(&mut guid) };

    let mut filter = CM_NOTIFY_FILTER {
        cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
        ..Default::default()
    };
    filter.u.DeviceInterface.ClassGuid = guid;

    let context = Box::into_raw(Box::new(Context { vid, f }));
    let mut handle: HCMNOTIFICATION = null_mut();
    // SAFETY: `filter` is fully initialised with cbSize set; `context` stays valid
    // until Registration::drop has unregistered; `handle` is a valid out pointer.
    let cr = unsafe {
        CM_Register_Notification(
            &filter,
            context.cast_const().cast(),
            Some(on_notify),
            &mut handle,
        )
    };
    if cr != CR_SUCCESS {
        // SAFETY: registration failed, so the system never kept `context`; freed once.
        drop(unsafe { Box::from_raw(context) });
        return Err(HidError::Io(io::Error::other(format!(
            "CM_Register_Notification failed (CONFIGRET {cr})"
        ))));
    }
    Ok(Registration { handle, context })
}

/// System callback: decode the symbolic link, filter on the vendor, hand off.
/// Does nothing else (no I/O, no locks beyond what the closure takes).
unsafe extern "system" fn on_notify(
    _hnotify: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    eventdata: *const CM_NOTIFY_EVENT_DATA,
    eventdatasize: u32,
) -> u32 {
    let arrived = match action {
        CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL => true,
        CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL => false,
        _ => return ERROR_SUCCESS,
    };
    if context.is_null() || eventdata.is_null() {
        return ERROR_SUCCESS;
    }
    // SAFETY: `context` is the Box<Context> passed to CM_Register_Notification; it
    // outlives every callback (Drop unregisters before freeing it).
    let ctx = unsafe { &*context.cast::<Context>() };
    // SAFETY: for interface actions the system passes `eventdatasize` readable bytes
    // at `eventdata`, holding the DeviceInterface variant whose SymbolicLink is a
    // NUL-terminated UTF-16 string inside that block. Reads stay within the size.
    let path = unsafe {
        let start = addr_of!((*eventdata).u.DeviceInterface.SymbolicLink).cast::<u16>();
        let offset = start as usize - eventdata as usize;
        let max = (eventdatasize as usize).saturating_sub(offset) / 2;
        let s = std::slice::from_raw_parts(start, max);
        let len = s.iter().position(|&c| c == 0).unwrap_or(max);
        String::from_utf16_lossy(&s[..len])
    };
    if !is_vendor_path(&path, ctx.vid) {
        return ERROR_SUCCESS;
    }
    let path = path.to_ascii_lowercase();
    (ctx.f)(if arrived {
        HotplugEvent::Arrived { path }
    } else {
        HotplugEvent::Removed { path }
    });
    ERROR_SUCCESS
}
