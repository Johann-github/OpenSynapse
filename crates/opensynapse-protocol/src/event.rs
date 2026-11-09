//! Hardware events (device → host, PROTOCOL25 parser). `vault/Protocol/Hardware Events.md`.
//!
//! Input byte 0 = record ID (inferred: equal to the HID input report ID). Records 5
//! (device) and 9 (secondary device on the same dongle) carry notifications:
//! byte 1 = event ID, payload from byte 2, 16-bit values big-endian.

use crate::be16;

/// Record ID of a notification from the primary device.
pub const RECORD_NOTIFICATION: u8 = 0x05;
/// Record ID of a notification from a dongle's secondary device.
pub const RECORD_SECONDARY_NOTIFICATION: u8 = 0x09;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    ProfileChanged(u8),
    DpiChanged {
        x: u16,
        y: u16,
    },
    Lifted,
    PutDown,
    /// `state`: 2 disconnect, 3 connect. `device_type`: 1 mouse, 2 keyboard, …
    WirelessChanged {
        state: u8,
        device_type: u8,
    },
    /// `level` 0–255, `charging` 0 no / 1 yes / 2 fault.
    Battery {
        level: u8,
        charging: u8,
        external_power: bool,
        device_type: u8,
    },
    ExternalPower(bool),
    ScrollModeChanged(u8),
    BatteryFull,
    /// 1 red … 4 green.
    ConnectionQuality(u8),
    /// `None` if the bitmask is not a single known rate.
    PollingRateChanged(Option<u32>),
    Other {
        id: u8,
        payload: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    /// From the dongle's secondary device (record 9) instead of the primary one.
    pub secondary: bool,
    pub event: Event,
}

/// Decode one input report (including its record/report ID byte).
/// Returns `None` for non-notification records or truncated input.
pub fn decode(record: &[u8]) -> Option<Notification> {
    let secondary = match *record.first()? {
        RECORD_NOTIFICATION => false,
        RECORD_SECONDARY_NOTIFICATION => true,
        _ => return None,
    };
    let id = *record.get(1)?;
    let p = &record[2..];
    let at = |i: usize| p.get(i).copied();
    let event = match id {
        0x01 => Event::ProfileChanged(at(0)?),
        0x02 if p.len() >= 4 => Event::DpiChanged {
            x: be16(p, 0),
            y: be16(p, 2),
        },
        0x05 => Event::Lifted,
        0x06 => Event::PutDown,
        0x09 => Event::WirelessChanged {
            state: at(0)?,
            device_type: at(1)?,
        },
        0x0C => Event::ExternalPower(at(0)? != 0),
        0x31 => Event::Battery {
            level: at(0)?,
            charging: at(1)?,
            external_power: at(2)? != 0,
            device_type: at(3)?,
        },
        0x39 => Event::ScrollModeChanged(at(0)?),
        0x41 => Event::BatteryFull,
        0x56 => Event::ConnectionQuality(at(0)?),
        0x58 => Event::PollingRateChanged(crate::polling::hyper_hz(at(0)?)),
        0x02 => return None,
        _ => Event::Other {
            id,
            payload: p.to_vec(),
        },
    };
    Some(Notification { secondary, event })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_change() {
        let n = decode(&[5, 0x02, 0x03, 0x20, 0x03, 0x20, 0, 0]).unwrap();
        assert!(!n.secondary);
        assert_eq!(n.event, Event::DpiChanged { x: 800, y: 800 });
    }

    #[test]
    fn battery() {
        let n = decode(&[5, 0x31, 255, 1, 1, 1]).unwrap();
        assert_eq!(
            n.event,
            Event::Battery {
                level: 255,
                charging: 1,
                external_power: true,
                device_type: 1
            }
        );
    }

    #[test]
    fn secondary_and_polling() {
        let n = decode(&[9, 0x58, 0x08]).unwrap();
        assert!(n.secondary);
        assert_eq!(n.event, Event::PollingRateChanged(Some(1000)));
        assert_eq!(
            decode(&[5, 0x58, 0x03]).unwrap().event,
            Event::PollingRateChanged(None)
        );
    }

    #[test]
    fn rejects_other_records_and_truncation() {
        assert_eq!(decode(&[2, 0xE9, 0x00]), None);
        assert_eq!(decode(&[5]), None);
        assert_eq!(decode(&[5, 0x02, 0x03]), None);
        assert_eq!(decode(&[5, 0x31, 1]), None);
        assert_eq!(decode(&[]), None);
    }

    #[test]
    fn unknown_event_kept() {
        assert_eq!(
            decode(&[5, 0x77, 1, 2]).unwrap().event,
            Event::Other {
                id: 0x77,
                payload: vec![1, 2]
            }
        );
    }
}
