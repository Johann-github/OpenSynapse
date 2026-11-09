//! Onboard profiles (class 0x05). `vault/Protocol/Commands 2.5.md`.

use crate::command::*;
use crate::{ProtocolError, Request, fixed, need};

pub fn get_profile_count() -> Request {
    fixed(GET_PROFILE_COUNT, &[])
}

pub fn get_max_profiles() -> Request {
    fixed(GET_MAX_PROFILES, &[])
}

pub fn get_active_profile() -> Request {
    fixed(GET_ACTIVE_PROFILE, &[])
}

pub fn set_active_profile(profile: u8) -> Request {
    fixed(SET_ACTIVE_PROFILE, &[profile])
}

/// All of the above reply with a single byte.
pub fn parse_byte(args: &[u8]) -> Result<u8, ProtocolError> {
    need(args, 1)?;
    Ok(args[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_profile() {
        let r = set_active_profile(2).encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x05, 0x04, 1));
        assert_eq!(r.args()[0], 2);
        assert_eq!(get_active_profile().encode(0).command_id(), 0x84);
        assert_eq!(parse_byte(&[3]).unwrap(), 3);
        assert!(parse_byte(&[]).is_err());
    }
}
