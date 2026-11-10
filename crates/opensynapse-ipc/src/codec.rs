//! Versioned binary encoding of [`crate::msg`] frames. Pure, std only.
//!
//! Integers little-endian; `bool` strictly 0/1; strings and lists carry a `u32`
//! length; `Option` tag 0/1; `Result` 0 = Ok / 1 = Err; every enum variant has an
//! explicit, never-reused tag. The decoder must consume the whole body.

use crate::msg::{ClientFrame, ServerFrame};

/// Why a frame body could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("message ends early")]
    Truncated,
    #[error("unknown tag {tag} for {what}")]
    UnknownTag { what: &'static str, tag: u8 },
    #[error("invalid bool byte {0}")]
    InvalidBool(u8),
    #[error("string is not valid UTF-8")]
    InvalidUtf8,
    #[error("length {0} exceeds the remaining bytes")]
    Length(u32),
    #[error("{0} trailing bytes after the message")]
    Trailing(usize),
    #[error("invalid value: {0}")]
    Invalid(String),
}

pub fn encode_client(f: &ClientFrame) -> Vec<u8> {
    let _ = f;
    todo!("ipc codec: encode_client")
}

pub fn decode_client(b: &[u8]) -> Result<ClientFrame, DecodeError> {
    let _ = b;
    todo!("ipc codec: decode_client")
}

pub fn encode_server(f: &ServerFrame) -> Vec<u8> {
    let _ = f;
    todo!("ipc codec: encode_server")
}

pub fn decode_server(b: &[u8]) -> Result<ServerFrame, DecodeError> {
    let _ = b;
    todo!("ipc codec: decode_server")
}
