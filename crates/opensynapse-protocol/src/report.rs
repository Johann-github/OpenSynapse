//! 90-byte report framing for protocol 2.5.
//!
//! Layout, CRC, status codes and transaction-ID rules: `vault/Protocol/Report Format 2.5.md`.

use crate::{Command, ProtocolError};

/// Razer payload length (without the HID report-ID byte).
pub const REPORT_LEN: usize = 90;
/// Feature report length as sent on the wire: report ID 0 + 90-byte payload.
pub const FEATURE_REPORT_LEN: usize = REPORT_LEN + 1;
/// Size of the argument area (offsets 8..88).
pub const ARGS_LEN: usize = 80;

const STATUS: usize = 0;
const TRANSACTION_ID: usize = 1;
const DATA_SIZE: usize = 5;
const CLASS: usize = 6;
const COMMAND_ID: usize = 7;
const ARGS: usize = 8;
const CRC: usize = 88;

/// Response status (byte 0). Observed values 0x00..=0x0B.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    NewCommand,
    Busy,
    Success,
    Failure,
    Timeout,
    NotSupported,
    ProfileNotSupported,
    TargetNotSupported,
    GrabControl,
    WrongDeviceMode,
    Broadcast,
    UnknownCommand,
    Other(u8),
}

/// What a host should do after reading a status (Synapse's observed handling).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// The command completed.
    Done,
    /// Device still working: read the report again.
    Poll,
    /// Transient failure: send the request again.
    Resend,
    /// Permanent failure: give up.
    Fail,
}

impl Status {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::NewCommand,
            0x01 => Self::Busy,
            0x02 => Self::Success,
            0x03 => Self::Failure,
            0x04 => Self::Timeout,
            0x05 => Self::NotSupported,
            0x06 => Self::ProfileNotSupported,
            0x07 => Self::TargetNotSupported,
            0x08 => Self::GrabControl,
            0x09 => Self::WrongDeviceMode,
            0x0A => Self::Broadcast,
            0x0B => Self::UnknownCommand,
            other => Self::Other(other),
        }
    }

    pub fn disposition(self) -> Disposition {
        match self {
            Self::Success => Disposition::Done,
            Self::Busy => Disposition::Poll,
            Self::NewCommand | Self::Failure | Self::Timeout => Disposition::Resend,
            _ => Disposition::Fail,
        }
    }
}

/// Rolling transaction counter: 0x00..=0x1E, wraps, never yields 0x1F (observed).
#[derive(Clone, Debug, Default)]
pub struct TransactionCounter {
    next: u8,
}

/// Highest value the rolling counter emits.
pub const TRANSACTION_COUNTER_MAX: u8 = 0x1E;
/// Routing bit for a secondary target behind a receiver (inferred meaning).
pub const ROUTE_SECONDARY: u8 = 0x80;

impl TransactionCounter {
    pub fn next_id(&mut self) -> u8 {
        let id = self.next;
        self.next = if id >= TRANSACTION_COUNTER_MAX {
            0
        } else {
            id + 1
        };
        id
    }
}

/// A command plus its arguments, not yet bound to a transaction ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    command: Command,
    data_size: u8,
    args: [u8; ARGS_LEN],
}

impl Request {
    /// Request with the command's default data size.
    pub fn new(command: Command, args: &[u8]) -> Result<Self, ProtocolError> {
        Self::with_size(command, command.size, args)
    }

    /// Request with an explicit data size (variable-length commands).
    pub fn with_size(command: Command, data_size: u8, args: &[u8]) -> Result<Self, ProtocolError> {
        if usize::from(data_size) > ARGS_LEN {
            return Err(ProtocolError::DataSize(data_size));
        }
        if args.len() > usize::from(data_size) {
            return Err(ProtocolError::ArgsTooLong {
                len: args.len(),
                data_size,
            });
        }
        let mut buf = [0u8; ARGS_LEN];
        buf[..args.len()].copy_from_slice(args);
        Ok(Self {
            command,
            data_size,
            args: buf,
        })
    }

    pub fn command(&self) -> Command {
        self.command
    }

    pub fn data_size(&self) -> u8 {
        self.data_size
    }

    /// Bind to a transaction ID and produce the wire report (CRC filled in).
    pub fn encode(&self, transaction_id: u8) -> Report {
        let mut b = [0u8; REPORT_LEN];
        b[STATUS] = 0x00;
        b[TRANSACTION_ID] = transaction_id;
        b[DATA_SIZE] = self.data_size;
        b[CLASS] = self.command.class;
        b[COMMAND_ID] = self.command.id;
        b[ARGS..ARGS + ARGS_LEN].copy_from_slice(&self.args);
        b[CRC] = crc(&b);
        Report(b)
    }
}

/// One 90-byte report (request or response).
#[derive(Clone, PartialEq, Eq)]
pub struct Report([u8; REPORT_LEN]);

impl Report {
    /// Parse a bare 90-byte payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let arr: [u8; REPORT_LEN] = bytes
            .try_into()
            .map_err(|_| ProtocolError::Length(bytes.len()))?;
        Ok(Self(arr))
    }

    /// Parse a 91-byte feature report buffer (byte 0 = report ID, dropped).
    pub fn from_feature_report(buf: &[u8]) -> Result<Self, ProtocolError> {
        if buf.len() != FEATURE_REPORT_LEN {
            return Err(ProtocolError::Length(buf.len()));
        }
        Self::from_bytes(&buf[1..])
    }

    /// 91-byte buffer for `send_feature_report`: report ID 0 + payload.
    pub fn to_feature_report(&self) -> [u8; FEATURE_REPORT_LEN] {
        let mut out = [0u8; FEATURE_REPORT_LEN];
        out[1..].copy_from_slice(&self.0);
        out
    }

    pub fn as_bytes(&self) -> &[u8; REPORT_LEN] {
        &self.0
    }

    pub fn status(&self) -> Status {
        Status::from_byte(self.0[STATUS])
    }

    pub fn transaction_id(&self) -> u8 {
        self.0[TRANSACTION_ID]
    }

    pub fn data_size(&self) -> u8 {
        self.0[DATA_SIZE]
    }

    pub fn class(&self) -> u8 {
        self.0[CLASS]
    }

    pub fn command_id(&self) -> u8 {
        self.0[COMMAND_ID]
    }

    /// Full 80-byte argument area. Parsers read from here instead of trusting
    /// `data_size`, because only the request side of `data_size` is confirmed.
    pub fn args(&self) -> &[u8] {
        &self.0[ARGS..ARGS + ARGS_LEN]
    }

    pub fn crc(&self) -> u8 {
        self.0[CRC]
    }

    pub fn crc_valid(&self) -> bool {
        crc(&self.0) == self.0[CRC]
    }

    /// True if `self` is the reply to `request`: same transaction ID, class and command ID.
    pub fn answers(&self, request: &Report) -> bool {
        self.transaction_id() == request.transaction_id()
            && self.class() == request.class()
            && self.command_id() == request.command_id()
    }
}

impl std::fmt::Debug for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Report {{ status: {:?}, tid: {:#04x}, cmd: {:02x}/{:02x}, size: {}, args: ",
            self.status(),
            self.transaction_id(),
            self.class(),
            self.command_id(),
            self.data_size()
        )?;
        let shown = usize::from(self.data_size()).clamp(1, ARGS_LEN);
        for b in &self.args()[..shown] {
            write!(f, "{b:02x}")?;
        }
        write!(f, ", crc: {:#04x} }}", self.crc())
    }
}

/// XOR of bytes 2..=87.
pub fn crc(report: &[u8; REPORT_LEN]) -> u8 {
    report[2..CRC].iter().fold(0, |acc, b| acc ^ b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FW: Command = Command::new(0x00, 0x81, 2);

    #[test]
    fn encode_layout() {
        let r = Request::new(FW, &[]).unwrap().encode(0x07);
        let b = r.as_bytes();
        assert_eq!(b[0], 0x00);
        assert_eq!(b[1], 0x07);
        assert_eq!(&b[2..5], &[0, 0, 0]);
        assert_eq!(b[5], 2);
        assert_eq!(b[6], 0x00);
        assert_eq!(b[7], 0x81);
        assert!(b[8..88].iter().all(|&x| x == 0));
        // CRC = 2 ^ 0x00 ^ 0x81
        assert_eq!(b[88], 0x83);
        assert_eq!(b[89], 0);
        assert!(r.crc_valid());
    }

    #[test]
    fn crc_excludes_status_and_tid() {
        let mut a = Request::new(FW, &[])
            .unwrap()
            .encode(0x00)
            .as_bytes()
            .to_owned();
        let c = crc(&a);
        a[0] = 0x02;
        a[1] = 0x1E;
        assert_eq!(crc(&a), c);
    }

    #[test]
    fn crc_covers_args() {
        let cmd = Command::new(0x04, 0x05, 7);
        let r = Request::new(cmd, &[0x01, 0x03, 0x20, 0x03, 0x20, 0, 0])
            .unwrap()
            .encode(0);
        // 7 ^ 04 ^ 05 ^ 01 ^ 03 ^ 20 ^ 03 ^ 20 = 0x07
        assert_eq!(r.crc(), 0x07);
    }

    #[test]
    fn args_longer_than_size_rejected() {
        assert!(matches!(
            Request::new(FW, &[1, 2, 3]),
            Err(ProtocolError::ArgsTooLong { len: 3, .. })
        ));
        assert!(Request::with_size(FW, 81, &[]).is_err());
    }

    #[test]
    fn feature_report_roundtrip() {
        let r = Request::new(FW, &[]).unwrap().encode(3);
        let wire = r.to_feature_report();
        assert_eq!(wire.len(), 91);
        assert_eq!(wire[0], 0);
        assert_eq!(Report::from_feature_report(&wire).unwrap(), r);
        assert!(Report::from_feature_report(&wire[..90]).is_err());
    }

    #[test]
    fn counter_wraps_before_0x1f() {
        let mut c = TransactionCounter::default();
        let ids: Vec<u8> = (0..33).map(|_| c.next_id()).collect();
        assert_eq!(ids[0], 0);
        assert_eq!(ids[30], 0x1E);
        assert_eq!(ids[31], 0);
        assert!(ids.iter().all(|&i| i <= 0x1E));
    }

    #[test]
    fn reply_matching() {
        let req = Request::new(FW, &[]).unwrap().encode(5);
        let mut resp = *req.as_bytes();
        resp[0] = 0x02;
        resp[8] = 1;
        resp[9] = 4;
        let resp = Report::from_bytes(&resp).unwrap();
        assert!(resp.answers(&req));
        assert_eq!(resp.status(), Status::Success);
        assert_eq!(&resp.args()[..2], &[1, 4]);
        let other = Request::new(FW, &[]).unwrap().encode(6);
        assert!(!resp.answers(&other));
    }

    #[test]
    fn status_dispositions() {
        assert_eq!(Status::from_byte(2).disposition(), Disposition::Done);
        assert_eq!(Status::from_byte(1).disposition(), Disposition::Poll);
        for b in [0, 3, 4] {
            assert_eq!(Status::from_byte(b).disposition(), Disposition::Resend);
        }
        for b in 5..=0x0B {
            assert_eq!(Status::from_byte(b).disposition(), Disposition::Fail);
        }
        assert_eq!(Status::from_byte(0x42), Status::Other(0x42));
    }
}
