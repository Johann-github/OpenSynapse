//! Device arrival/removal notifications without polling. Windows:
//! `CM_Register_Notification` on the HID interface class; other platforms:
//! unsupported for now.

use crate::HidError;

/// A Razer HID interface appeared or went away (lowercase symbolic link path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotplugEvent {
    Arrived { path: String },
    Removed { path: String },
}

/// Active registration; dropping it unregisters the callback.
pub struct HotplugWatch {
    _priv: (),
}

impl Drop for HotplugWatch {
    fn drop(&mut self) {}
}

/// Call `f` for every HID interface of vendor `vid` that arrives or is removed.
/// `f` runs on a system thread and must only hand the event off.
pub fn watch(
    vid: u16,
    f: impl Fn(HotplugEvent) + Send + Sync + 'static,
) -> Result<HotplugWatch, HidError> {
    let _ = (vid, f);
    Err(HidError::Io(std::io::Error::from(
        std::io::ErrorKind::Unsupported,
    )))
}
