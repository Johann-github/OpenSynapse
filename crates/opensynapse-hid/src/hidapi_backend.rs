//! hidapi backend for Linux/macOS. Not used on Windows (see `windows.rs`).

use std::ffi::CString;
use std::io;

use crate::{FeatureIo, HidEntry, HidError};

fn io_err(e: hidapi::HidError) -> io::Error {
    io::Error::other(e.to_string())
}

pub struct Device(hidapi::HidDevice);

impl Device {
    pub fn open(path: &str) -> io::Result<Self> {
        let api = hidapi::HidApi::new().map_err(io_err)?;
        let c = CString::new(path).map_err(io::Error::other)?;
        api.open_path(&c).map(Self).map_err(io_err)
    }
}

impl FeatureIo for Device {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError> {
        self.0
            .send_feature_report(buf)
            .map_err(|e| io_err(e).into())
    }

    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError> {
        self.0.get_feature_report(buf).map_err(|e| io_err(e).into())
    }
}

pub fn enumerate(vid: u16) -> io::Result<Vec<HidEntry>> {
    let api = hidapi::HidApi::new().map_err(io_err)?;
    Ok(api
        .device_list()
        .filter(|d| d.vendor_id() == vid)
        .map(|d| HidEntry {
            vid: d.vendor_id(),
            pid: d.product_id(),
            interface: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            feature_len: None,
            path: d.path().to_string_lossy().into_owned(),
        })
        .collect())
}

/// Razer software does not run on these platforms.
pub fn process_running(_image: &str) -> bool {
    false
}
