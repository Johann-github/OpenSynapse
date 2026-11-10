//! Windows backend: SetupAPI enumeration + hid.dll feature reports.
//!
//! Why not hidapi here: `hidapi::HidApi::new()` opens and queries every HID
//! collection on the system (~200 ms on the dev PC), which breaks the `list`
//! budget. This backend filters on `vid_1532` in the interface path *before*
//! opening anything. Decision: `vault/Decisions/ADR-0003 Own Windows HID backend.md`.
//!
//! Handles are opened with zero access rights: the OS owns the mouse collection,
//! and feature reports work on such a handle (`vault/Protocol/Windows Transport.md`).

#![allow(unsafe_code)] // FFI only; every block states its invariant.

use std::io;
use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS, HidD_FreePreparsedData, HidD_GetAttributes,
    HidD_GetFeature, HidD_GetHidGuid, HidD_GetPreparsedData, HidD_SetFeature, HidP_GetCaps,
};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::core::GUID;

use crate::{FeatureIo, HidEntry, HidError};

/// An owned, zero-access handle to one HID collection.
pub struct Device {
    handle: HANDLE,
}

// SAFETY: a file HANDLE may be used from any thread; Session serialises access.
unsafe impl Send for Device {}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: `handle` came from a successful CreateFileW and is closed once.
        unsafe { CloseHandle(self.handle) };
    }
}

impl Device {
    pub fn open(path: &str) -> io::Result<Self> {
        open_raw(path, 0, 0).map(|handle| Self { handle })
    }

    fn attributes(&self) -> Option<HIDD_ATTRIBUTES> {
        let mut a = HIDD_ATTRIBUTES {
            Size: size_of::<HIDD_ATTRIBUTES>() as u32,
            VendorID: 0,
            ProductID: 0,
            VersionNumber: 0,
        };
        // SAFETY: valid handle, `a` is a properly sized out-struct.
        unsafe { HidD_GetAttributes(self.handle, &mut a) }.then_some(a)
    }

    fn caps(&self) -> Option<HIDP_CAPS> {
        caps_of(self.handle)
    }
}

/// `CreateFileW` on `path`, shared read/write, with these access rights and flags.
/// The caller owns (and must close) the returned handle.
pub(crate) fn open_raw(path: &str, access: u32, flags: u32) -> io::Result<HANDLE> {
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(handle)
}

/// HidP_GetCaps of an open HID handle (any access rights).
pub(crate) fn caps_of(handle: HANDLE) -> Option<HIDP_CAPS> {
    let mut pp = 0;
    // SAFETY: valid handle; on success `pp` must be freed with HidD_FreePreparsedData.
    if !unsafe { HidD_GetPreparsedData(handle, &mut pp) } {
        return None;
    }
    // SAFETY: HIDP_CAPS is plain data; all-zero is a valid value.
    let mut caps: HIDP_CAPS = unsafe { std::mem::zeroed() };
    // SAFETY: `pp` is valid until freed below.
    let status = unsafe { HidP_GetCaps(pp, &mut caps) };
    // SAFETY: `pp` came from HidD_GetPreparsedData and is freed once.
    unsafe { HidD_FreePreparsedData(pp) };
    (status == HIDP_STATUS_SUCCESS).then_some(caps)
}

/// True if a shared `GENERIC_READ` open of `path` succeeds (closed right away).
pub fn can_read(path: &str) -> bool {
    match open_raw(path, GENERIC_READ, 0) {
        Ok(h) => {
            // SAFETY: `h` came from a successful CreateFileW and is closed once.
            unsafe { CloseHandle(h) };
            true
        }
        Err(_) => false,
    }
}

impl FeatureIo for Device {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError> {
        // SAFETY: `buf` is valid for `buf.len()` bytes for the duration of the call.
        let ok = unsafe { HidD_SetFeature(self.handle, buf.as_ptr().cast(), buf.len() as u32) };
        if ok {
            Ok(())
        } else {
            Err(io::Error::last_os_error().into())
        }
    }

    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError> {
        // SAFETY: `buf` is valid and writable for `buf.len()` bytes; byte 0 holds the report ID.
        let ok = unsafe { HidD_GetFeature(self.handle, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if ok {
            Ok(buf.len())
        } else {
            Err(io::Error::last_os_error().into())
        }
    }
}

/// Owned SetupAPI device-info set.
struct DevInfo(isize);

impl Drop for DevInfo {
    fn drop(&mut self) {
        // SAFETY: the set came from a successful SetupDiGetClassDevsW.
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

/// Every present HID interface path (no device is opened).
fn interface_paths() -> io::Result<Vec<String>> {
    let mut guid = GUID::from_u128(0);
    // SAFETY: writes one GUID.
    unsafe { HidD_GetHidGuid(&mut guid) };
    // SAFETY: valid GUID pointer; null enumerator/window are allowed.
    let set = unsafe {
        SetupDiGetClassDevsW(
            &guid,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == INVALID_HANDLE_VALUE as isize {
        return Err(io::Error::last_os_error());
    }
    let set = DevInfo(set);
    let mut paths = Vec::new();
    for index in 0.. {
        let mut data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            InterfaceClassGuid: GUID::from_u128(0),
            Flags: 0,
            Reserved: 0,
        };
        // SAFETY: valid set and out-struct with cbSize set.
        if unsafe { SetupDiEnumDeviceInterfaces(set.0, null(), &guid, index, &mut data) } == 0 {
            break;
        }
        let mut needed = 0u32;
        // SAFETY: size query: null buffer with length 0 is the documented pattern.
        unsafe {
            SetupDiGetDeviceInterfaceDetailW(set.0, &data, null_mut(), 0, &mut needed, null_mut())
        };
        if (needed as usize) < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() {
            continue;
        }
        // u32 storage keeps the struct 4-byte aligned.
        let mut buf = vec![0u32; (needed as usize).div_ceil(4)];
        let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        // SAFETY: `buf` holds at least `needed` bytes and is aligned for the struct.
        let ok = unsafe {
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            SetupDiGetDeviceInterfaceDetailW(set.0, &data, detail, needed, null_mut(), null_mut())
        };
        if ok == 0 {
            continue;
        }
        // SAFETY: DevicePath starts at offset 4 and runs NUL-terminated within `needed` bytes.
        let path = unsafe {
            let start = std::ptr::addr_of!((*detail).DevicePath).cast::<u16>();
            let max = (needed as usize - 4) / 2;
            let s = std::slice::from_raw_parts(start, max);
            let len = s.iter().position(|&c| c == 0).unwrap_or(max);
            String::from_utf16_lossy(&s[..len])
        };
        paths.push(path);
    }
    Ok(paths)
}

/// `&mi_XX` in the path → interface number; -1 if the device is not composite.
fn interface_from_path(lower: &str) -> i32 {
    lower
        .find("&mi_")
        .and_then(|i| lower.get(i + 4..i + 6))
        .and_then(|h| i32::from_str_radix(h, 16).ok())
        .unwrap_or(-1)
}

/// Collections of devices with this vendor ID (every HID collection with
/// `None`). Only matching paths are opened (zero access).
pub fn enumerate_any(vid: Option<u16>) -> io::Result<Vec<HidEntry>> {
    let needle = vid.map(|v| format!("vid_{v:04x}"));
    let mut out = Vec::new();
    for path in interface_paths()? {
        let lower = path.to_ascii_lowercase();
        if needle.as_ref().is_some_and(|n| !lower.contains(n.as_str())) {
            continue;
        }
        let Ok(dev) = Device::open(&path) else {
            continue;
        };
        let Some(attr) = dev.attributes() else {
            continue;
        };
        let caps = dev.caps();
        out.push(HidEntry {
            vid: attr.VendorID,
            pid: attr.ProductID,
            interface: interface_from_path(&lower),
            usage_page: caps.map_or(0, |c| c.UsagePage),
            usage: caps.map_or(0, |c| c.Usage),
            feature_len: caps.map(|c| c.FeatureReportByteLength),
            input_len: caps.map(|c| c.InputReportByteLength),
            output_len: caps.map(|c| c.OutputReportByteLength),
            path,
        });
    }
    Ok(out)
}

/// True if a process with this image name runs (case-insensitive).
pub fn process_running(image: &str) -> bool {
    // SAFETY: snapshot of all processes; closed below.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return false;
    }
    // SAFETY: PROCESSENTRY32W is plain data; all-zero is valid before dwSize is set.
    let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut found = false;
    // SAFETY: valid snapshot handle and sized out-struct.
    let mut more = unsafe { Process32FirstW(snap, &mut e) } != 0;
    while more {
        let len = e
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(e.szExeFile.len());
        if String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case(image) {
            found = true;
            break;
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snap, &mut e) } != 0;
    }
    // SAFETY: `snap` came from CreateToolhelp32Snapshot and is closed once.
    unsafe { CloseHandle(snap) };
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_number_from_path() {
        assert_eq!(
            interface_from_path(r"\\?\hid#vid_1532&pid_00b8&mi_00#7&1a2b&0&0000#{4d1e55b2}"),
            0
        );
        assert_eq!(
            interface_from_path(r"\\?\hid#vid_1532&pid_00b8&mi_01&col02#8&x#{guid}"),
            1
        );
        assert_eq!(interface_from_path(r"\\?\hid#vid_046d&pid_c52b#x"), -1);
    }

    #[test]
    fn own_process_is_found() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_str().unwrap();
        assert!(process_running(name));
        assert!(!process_running("definitely-not-running-opensynapse.exe"));
    }

    #[test]
    fn enumeration_runs() {
        // Smoke test: must not fail or open non-matching devices.
        let all = enumerate_any(Some(0x1532)).unwrap();
        assert!(all.iter().all(|e| e.vid == 0x1532));
    }
}
