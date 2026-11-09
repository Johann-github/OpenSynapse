//! Device information and mode (class 0x00). `vault/Protocol/Commands 2.5.md`.

use crate::command::*;
use crate::{ProtocolError, Request, fixed, need};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirmwareVersion {
    pub major: u8,
    pub minor: u8,
}

impl std::fmt::Display for FirmwareVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{:02}", self.major, self.minor)
    }
}

pub fn get_firmware_version() -> Request {
    fixed(GET_FIRMWARE_VERSION, &[])
}

pub fn parse_firmware_version(args: &[u8]) -> Result<FirmwareVersion, ProtocolError> {
    need(args, 2)?;
    Ok(FirmwareVersion {
        major: args[0],
        minor: args[1],
    })
}

pub fn get_protocol_version() -> Request {
    fixed(GET_PROTOCOL_VERSION, &[])
}

/// Same `[major, minor]` layout as the firmware version.
pub fn parse_protocol_version(args: &[u8]) -> Result<FirmwareVersion, ProtocolError> {
    parse_firmware_version(args)
}

pub fn get_serial() -> Request {
    fixed(GET_SERIAL, &[])
}

/// 22 bytes of ASCII; stops at the first NUL, drops non-printable bytes.
pub fn parse_serial(args: &[u8]) -> Result<String, ProtocolError> {
    need(args, 22)?;
    Ok(args[..22]
        .iter()
        .take_while(|&&b| b != 0)
        .filter(|b| b.is_ascii_graphic() || **b == b' ')
        .map(|&b| char::from(b))
        .collect::<String>()
        .trim()
        .to_owned())
}

/// Device mode byte (observed values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceMode {
    Normal,
    Bootloader,
    Testing,
    Driver,
    Download,
    Pairing,
    Standby,
    RfTest,
    Retail,
    Other(u8),
}

impl DeviceMode {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Normal,
            1 => Self::Bootloader,
            2 => Self::Testing,
            3 => Self::Driver,
            4 => Self::Download,
            5 => Self::Pairing,
            6 => Self::Standby,
            8 => Self::RfTest,
            9 => Self::Retail,
            o => Self::Other(o),
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Bootloader => 1,
            Self::Testing => 2,
            Self::Driver => 3,
            Self::Download => 4,
            Self::Pairing => 5,
            Self::Standby => 6,
            Self::RfTest => 8,
            Self::Retail => 9,
            Self::Other(o) => o,
        }
    }
}

pub fn get_device_mode() -> Request {
    fixed(GET_DEVICE_MODE, &[])
}

/// `[mode, param]`.
pub fn set_device_mode(mode: DeviceMode, param: u8) -> Request {
    fixed(SET_DEVICE_MODE, &[mode.to_byte(), param])
}

pub fn parse_device_mode(args: &[u8]) -> Result<(DeviceMode, u8), ProtocolError> {
    need(args, 2)?;
    Ok((DeviceMode::from_byte(args[0]), args[1]))
}

pub fn get_wireless_status() -> Request {
    fixed(GET_WIRELESS_STATUS, &[])
}

/// `[connected]`.
pub fn parse_wireless_status(args: &[u8]) -> Result<bool, ProtocolError> {
    need(args, 1)?;
    Ok(args[0] != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmware() {
        let r = get_firmware_version().encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x00, 0x81, 2));
        let v = parse_firmware_version(&[1, 4, 0, 0]).unwrap();
        assert_eq!(v.to_string(), "1.04");
        assert!(parse_firmware_version(&[1]).is_err());
    }

    #[test]
    fn serial() {
        let mut a = [0u8; 80];
        a[..12].copy_from_slice(b"PM2412345678");
        assert_eq!(parse_serial(&a).unwrap(), "PM2412345678");
        a[3] = 0x07;
        assert_eq!(parse_serial(&a).unwrap(), "PM212345678");
    }

    #[test]
    fn device_mode() {
        let r = set_device_mode(DeviceMode::Driver, 0).encode(1);
        assert_eq!(&r.args()[..2], &[3, 0]);
        assert_eq!(r.command_id(), 0x04);
        assert_eq!(parse_device_mode(&[0, 0]).unwrap(), (DeviceMode::Normal, 0));
        for b in 0..=10 {
            assert_eq!(DeviceMode::from_byte(b).to_byte(), b);
        }
    }
}
