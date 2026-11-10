//! Device arrival/removal notifications without polling. Windows:
//! `CM_Register_Notification` on the HID interface class (`windows_hotplug.rs`);
//! other platforms: unsupported for now.
//!
//! Decision: `vault/Decisions/ADR-0005 Agent IPC.md` (hotplug section).

use crate::HidError;

/// A Razer HID interface appeared or went away (lowercase symbolic link path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotplugEvent {
    Arrived { path: String },
    Removed { path: String },
}

/// The closure registered with [`watch`].
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) type Callback = Box<dyn Fn(HotplugEvent) + Send + Sync>;

/// Active registration; dropping it unregisters the callback (and waits for a
/// callback that is running at that moment).
pub struct HotplugWatch {
    #[cfg(windows)]
    _reg: crate::windows_hotplug::Registration,
    #[cfg(not(windows))]
    _priv: (),
}

/// True if an interface path belongs to vendor `vid` (`vid_1532` in the path,
/// any case). Same rule as the Windows enumeration filter.
pub fn is_vendor_path(path: &str, vid: u16) -> bool {
    let needle = format!("vid_{vid:04x}");
    path.to_ascii_lowercase().contains(&needle)
}

/// Call `f` for every HID interface of vendor `vid` that arrives or is removed.
/// `f` runs on a system thread and must only hand the event off.
pub fn watch(
    vid: u16,
    f: impl Fn(HotplugEvent) + Send + Sync + 'static,
) -> Result<HotplugWatch, HidError> {
    #[cfg(windows)]
    {
        let reg = crate::windows_hotplug::register(vid, Box::new(f))?;
        Ok(HotplugWatch { _reg: reg })
    }
    #[cfg(not(windows))]
    {
        let _ = (vid, f);
        Err(HidError::Io(std::io::Error::from(
            std::io::ErrorKind::Unsupported,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_path_matches_any_case() {
        let vid = 0x1532;
        assert!(is_vendor_path(
            r"\\?\hid#vid_1532&pid_00b8&mi_00#7&1a2b&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
            vid
        ));
        assert!(is_vendor_path(r"\\?\HID#VID_1532&PID_00B8&MI_00#x", vid));
        assert!(!is_vendor_path(r"\\?\hid#vid_046d&pid_c52b#x", vid));
        assert!(!is_vendor_path(r"\\?\hid#pid_1532#x", vid));
        assert!(!is_vendor_path("", vid));
        assert!(is_vendor_path(r"\\?\hid#vid_0001&pid_0002#x", 0x0001));
    }

    #[cfg(windows)]
    #[test]
    fn register_and_drop() {
        // Smoke test: registration succeeds and unregisters cleanly.
        let w = watch(0x1532, |_| {}).expect("register hotplug notification");
        drop(w);
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_elsewhere() {
        assert!(watch(0x1532, |_| {}).is_err());
    }
}
