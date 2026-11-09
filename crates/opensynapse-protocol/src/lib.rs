//! Pure encode/decode of Razer protocol 2.5 reports. No I/O. See README.md.
//!
//! Each typed module offers request builders (`get_*` / `set_*` → [`Request`]) and
//! response parsers (`parse_*(args)`), where `args` is [`Report::args`] of the reply.

pub mod buttons;
pub mod command;
pub mod device;
pub mod dpi;
pub mod event;
pub mod lighting;
pub mod polling;
pub mod power;
pub mod profile;
pub mod report;

pub use command::Command;
pub use report::{
    ARGS_LEN, Disposition, FEATURE_REPORT_LEN, REPORT_LEN, Report, Request, Status,
    TransactionCounter,
};

/// Encode/decode errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("report has {0} bytes, expected 90 (or 91 with report ID)")]
    Length(usize),
    #[error("data_size {0} exceeds the 80-byte argument area")]
    DataSize(u8),
    #[error("{len} argument bytes exceed data_size {data_size}")]
    ArgsTooLong { len: usize, data_size: u8 },
    #[error("response too short: need {need} argument bytes, got {got}")]
    Short { need: usize, got: usize },
    #[error("unexpected value {value:#04x} for {field}")]
    Value { field: &'static str, value: u8 },
    #[error("{0} is out of range")]
    Range(&'static str),
}

/// Fail with [`ProtocolError::Short`] unless `args` has at least `n` bytes.
pub(crate) fn need(args: &[u8], n: usize) -> Result<(), ProtocolError> {
    if args.len() < n {
        Err(ProtocolError::Short {
            need: n,
            got: args.len(),
        })
    } else {
        Ok(())
    }
}

pub(crate) fn be16(args: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([args[at], args[at + 1]])
}

/// Build a request whose argument layout is fixed by this crate, so it cannot fail.
pub(crate) fn fixed(command: Command, args: &[u8]) -> Request {
    Request::new(command, args).expect("argument layout fits the command's data_size")
}
