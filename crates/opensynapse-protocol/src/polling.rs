//! Polling rate (class 0x00). Two variants exist; which one a device supports is
//! unknown up front (`vault/Protocol/Commands 2.5.md`, open question for the Viper).
//!
//! - Period (00/05, 00/85): one code byte, rate = 1000 / code Hz.
//! - HyperPolling (00/40, 00/C0): `[profile, bitmask]`, one bit per rate.

use crate::command::*;
use crate::{ProtocolError, Request, fixed, need};

/// (code, Hz) pairs for the period variant (observed).
const PERIOD_CODES: [(u8, u32); 7] = [
    (1, 1000),
    (2, 500),
    (3, 333),
    (4, 250),
    (5, 200),
    (8, 125),
    (10, 100),
];

/// (bit, Hz) pairs for the HyperPolling variant (observed).
const HYPER_BITS: [(u8, u32); 7] = [
    (0x01, 8000),
    (0x02, 4000),
    (0x04, 2000),
    (0x08, 1000),
    (0x10, 500),
    (0x20, 250),
    (0x40, 125),
];

pub fn get_polling_rate() -> Request {
    fixed(GET_POLLING_PERIOD, &[])
}

pub fn set_polling_rate(hz: u32) -> Result<Request, ProtocolError> {
    let code = PERIOD_CODES
        .iter()
        .find(|(_, h)| *h == hz)
        .map(|(c, _)| *c)
        .ok_or(ProtocolError::Range("polling rate"))?;
    Ok(fixed(SET_POLLING_PERIOD, &[code]))
}

/// Reply `[code]` → Hz.
pub fn parse_polling_rate(args: &[u8]) -> Result<u32, ProtocolError> {
    need(args, 1)?;
    PERIOD_CODES
        .iter()
        .find(|(c, _)| *c == args[0])
        .map(|(_, h)| *h)
        .ok_or(ProtocolError::Value {
            field: "polling period code",
            value: args[0],
        })
}

pub fn get_hyper_polling(profile: u8) -> Request {
    fixed(GET_HYPER_POLLING, &[profile])
}

pub fn set_hyper_polling(profile: u8, hz: u32) -> Result<Request, ProtocolError> {
    let bit = hyper_bit(hz).ok_or(ProtocolError::Range("polling rate"))?;
    Ok(fixed(SET_HYPER_POLLING, &[profile, bit]))
}

/// Reply `[profile, bitmask]` → Hz. Exactly one bit must be set.
pub fn parse_hyper_polling(args: &[u8]) -> Result<u32, ProtocolError> {
    need(args, 2)?;
    hyper_hz(args[1]).ok_or(ProtocolError::Value {
        field: "HyperPolling bitmask",
        value: args[1],
    })
}

/// Hz for a single HyperPolling bit (also used by the polling-rate event).
pub fn hyper_hz(bit: u8) -> Option<u32> {
    HYPER_BITS.iter().find(|(b, _)| *b == bit).map(|(_, h)| *h)
}

pub fn hyper_bit(hz: u32) -> Option<u8> {
    HYPER_BITS.iter().find(|(_, h)| *h == hz).map(|(b, _)| *b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_variant() {
        let r = set_polling_rate(500).unwrap().encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0, 0x05, 1));
        assert_eq!(r.args()[0], 2);
        assert_eq!(parse_polling_rate(&[8]).unwrap(), 125);
        assert_eq!(parse_polling_rate(&[1]).unwrap(), 1000);
        assert!(parse_polling_rate(&[7]).is_err());
        assert!(set_polling_rate(8000).is_err());
    }

    #[test]
    fn hyper_variant() {
        let r = set_hyper_polling(1, 1000).unwrap().encode(0);
        assert_eq!((r.command_id(), r.data_size()), (0x40, 2));
        assert_eq!(&r.args()[..2], &[1, 0x08]);
        assert_eq!(get_hyper_polling(1).encode(0).command_id(), 0xC0);
        assert_eq!(parse_hyper_polling(&[1, 0x01]).unwrap(), 8000);
        assert!(parse_hyper_polling(&[1, 0x03]).is_err());
        assert!(set_hyper_polling(1, 333).is_err());
    }
}
