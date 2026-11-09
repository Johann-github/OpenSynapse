//! Battery, charging and sleep (class 0x07). `vault/Protocol/Commands 2.5.md`.

use crate::command::*;
use crate::{ProtocolError, Request, be16, fixed, need};

/// Battery index sent in battery requests. Single-battery mice use 0 (inferred).
pub const MAIN_BATTERY: u8 = 0;

/// Raw battery level 0–255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryLevel(pub u8);

impl BatteryLevel {
    /// Percent the way Synapse shows it: ⌊raw / 255 · 100⌋ (observed).
    pub fn percent(self) -> u8 {
        (u16::from(self.0) * 100 / 255) as u8
    }
}

pub fn get_battery_level() -> Request {
    fixed(GET_BATTERY_LEVEL, &[MAIN_BATTERY])
}

/// Reply `[batteryId, level]`.
pub fn parse_battery_level(args: &[u8]) -> Result<BatteryLevel, ProtocolError> {
    need(args, 2)?;
    Ok(BatteryLevel(args[1]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargingStatus {
    NotCharging,
    Charging,
    Failed,
    ForeignObject,
    FastCharging,
    Full,
    LimitReached,
    Other(u8),
}

impl ChargingStatus {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::NotCharging,
            1 => Self::Charging,
            2 => Self::Failed,
            3 => Self::ForeignObject,
            4 => Self::FastCharging,
            5 => Self::Full,
            6 => Self::LimitReached,
            o => Self::Other(o),
        }
    }
}

impl std::fmt::Display for ChargingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCharging => f.write_str("not charging"),
            Self::Charging => f.write_str("charging"),
            Self::Failed => f.write_str("charging failed"),
            Self::ForeignObject => f.write_str("foreign object detected"),
            Self::FastCharging => f.write_str("fast charging"),
            Self::Full => f.write_str("full"),
            Self::LimitReached => f.write_str("charge limit reached"),
            Self::Other(o) => write!(f, "unknown ({o})"),
        }
    }
}

pub fn get_charging_status() -> Request {
    fixed(GET_CHARGING_STATUS, &[MAIN_BATTERY])
}

/// Reply `[batteryId, status]`.
pub fn parse_charging_status(args: &[u8]) -> Result<ChargingStatus, ProtocolError> {
    need(args, 2)?;
    Ok(ChargingStatus::from_byte(args[1]))
}

pub fn get_low_battery_threshold() -> Request {
    fixed(GET_LOW_BATTERY_THRESHOLD, &[])
}

/// Percent → raw as ⌈p / 100 · 255⌉ (observed).
pub fn set_low_battery_threshold(percent: u8) -> Result<Request, ProtocolError> {
    if percent > 100 {
        return Err(ProtocolError::Range("low-battery threshold percent"));
    }
    let raw = (u16::from(percent) * 255).div_ceil(100) as u8;
    Ok(fixed(SET_LOW_BATTERY_THRESHOLD, &[raw]))
}

/// Reply `[raw]` → percent, rounded down.
pub fn parse_low_battery_threshold(args: &[u8]) -> Result<u8, ProtocolError> {
    need(args, 1)?;
    Ok(BatteryLevel(args[0]).percent())
}

pub fn get_sleep_time() -> Request {
    fixed(GET_SLEEP_TIME, &[])
}

/// `[time u16]`. Unit is **unknown** (seconds inferred).
pub fn set_sleep_time(raw: u16) -> Request {
    fixed(SET_SLEEP_TIME, &raw.to_be_bytes())
}

pub fn parse_sleep_time(args: &[u8]) -> Result<u16, ProtocolError> {
    need(args, 2)?;
    Ok(be16(args, 0))
}

pub fn get_connection_quality() -> Request {
    fixed(GET_CONNECTION_QUALITY, &[])
}

/// Reply `[quality]`, 1 (worst) … 4 (best) (inferred scale).
pub fn parse_connection_quality(args: &[u8]) -> Result<u8, ProtocolError> {
    need(args, 1)?;
    Ok(args[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery() {
        let r = get_battery_level().encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x07, 0x80, 2));
        let b = parse_battery_level(&[0, 255]).unwrap();
        assert_eq!(b.percent(), 100);
        assert_eq!(BatteryLevel(128).percent(), 50);
        assert_eq!(BatteryLevel(2).percent(), 0);
    }

    #[test]
    fn charging() {
        assert_eq!(
            parse_charging_status(&[0, 1]).unwrap(),
            ChargingStatus::Charging
        );
        assert_eq!(
            parse_charging_status(&[0, 9]).unwrap(),
            ChargingStatus::Other(9)
        );
    }

    #[test]
    fn low_battery() {
        assert_eq!(
            set_low_battery_threshold(30).unwrap().encode(0).args()[0],
            77
        );
        assert_eq!(
            set_low_battery_threshold(100).unwrap().encode(0).args()[0],
            255
        );
        assert_eq!(set_low_battery_threshold(0).unwrap().encode(0).args()[0], 0);
        assert!(set_low_battery_threshold(101).is_err());
        assert_eq!(parse_low_battery_threshold(&[77]).unwrap(), 30);
    }

    #[test]
    fn sleep() {
        let r = set_sleep_time(300).encode(0);
        assert_eq!((r.command_id(), r.data_size()), (0x03, 2));
        assert_eq!(&r.args()[..2], &[0x01, 0x2C]);
        assert_eq!(parse_sleep_time(&[0x01, 0x2C]).unwrap(), 300);
    }
}
