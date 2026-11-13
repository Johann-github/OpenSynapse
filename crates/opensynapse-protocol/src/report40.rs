//! 64-byte report framing for protocol 4.0 (Synapse class `rzDevice40`).
//!
//! Protocol 4.0 reuses the 2.5 command tables (same class / id / data size) in a
//! smaller frame: HID feature report ID 2, 63 payload bytes. Facts, evidence and
//! confidence: `vault/Protocol/Protocol 3.0 and 4.0.md`. Not hardware-tested.

use crate::report::{self, Report, Status};
use crate::{ProtocolError, Request};

/// HID report ID of the 4.0 feature report (observed).
pub const REPORT_ID: u8 = 2;
/// Payload length without the report-ID byte (observed).
pub const FRAME_LEN: usize = 63;
/// Feature report length on the wire: report ID 2 + 63-byte payload (observed).
pub const FEATURE_REPORT_LEN: usize = FRAME_LEN + 1;
/// Size of the argument area (offsets 8..61).
pub const ARGS_LEN: usize = 53;

const STATUS: usize = 0;
const TRANSACTION: usize = 1;
const DATA_SIZE_HI: usize = 4;
const DATA_SIZE_LO: usize = 5;
const CLASS: usize = 6;
const COMMAND_ID: usize = 7;
const ARGS: usize = 8;
const CHECKSUM: usize = 61;

/// Highest value of the 5-bit transaction counter (observed: 1..=31, wraps to 1).
pub const TRANSACTION_COUNTER_MAX: u8 = 31;
const COUNTER_MASK: u8 = 0x1F;
const TARGET_SHIFT: u8 = 5;

/// Target device type in transaction bits 5..7 (observed enum). The base class
/// sends everything to [`Target::Mouse`] unless a command names another target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Target {
    #[default]
    Mouse = 0,
    GameController1 = 1,
    GameController2 = 2,
    /// Headset / speaker / microphone.
    Audio = 3,
    Keyboard = 4,
    GameController3 = 5,
    GameController4 = 6,
    /// The receiver or dock itself.
    DockDongle = 7,
}

impl Target {
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0x07 {
            0 => Self::Mouse,
            1 => Self::GameController1,
            2 => Self::GameController2,
            3 => Self::Audio,
            4 => Self::Keyboard,
            5 => Self::GameController3,
            6 => Self::GameController4,
            _ => Self::DockDongle,
        }
    }
}

/// Rolling 4.0 transaction counter: 1..=31, wraps back to 1, never 0 (observed).
/// Differs from the 2.5 counter, which runs 0..=0x1E.
#[derive(Clone, Debug, Default)]
pub struct TransactionCounter {
    last: u8,
}

impl TransactionCounter {
    /// Next transaction byte for `target`: counter in bits 0..4, target in bits 5..7.
    pub fn next_id(&mut self, target: Target) -> u8 {
        if self.last >= TRANSACTION_COUNTER_MAX {
            self.last = 0;
        }
        self.last += 1;
        self.last | ((target as u8) << TARGET_SHIFT)
    }
}

/// One 63-byte 4.0 frame (request or response), without the report-ID byte.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame([u8; FRAME_LEN]);

impl Frame {
    /// Bind a (2.5-table) request to a transaction byte and build the 4.0 frame.
    /// Fails when the request's data size exceeds the 53-byte argument area.
    pub fn encode(request: &Request, transaction: u8) -> Result<Self, ProtocolError> {
        let size = request.data_size();
        if usize::from(size) > ARGS_LEN {
            return Err(ProtocolError::FrameDataSize {
                size,
                max: ARGS_LEN,
            });
        }
        let mut b = [0u8; FRAME_LEN];
        b[STATUS] = 0x00;
        b[TRANSACTION] = transaction;
        // Bytes 2..4 stay 0: Synapse fills them only for sizes > 255, which a
        // u8 data size cannot reach. Byte 4 is the size's high byte (0 here).
        b[DATA_SIZE_LO] = size;
        b[CLASS] = request.command().class;
        b[COMMAND_ID] = request.command().id;
        let n = usize::from(size);
        b[ARGS..ARGS + n].copy_from_slice(&request.args()[..n]);
        b[CHECKSUM] = checksum(&b);
        Ok(Self(b))
    }

    /// Parse a bare 63-byte payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let arr: [u8; FRAME_LEN] = bytes.try_into().map_err(|_| ProtocolError::FrameLength {
            got: bytes.len(),
            expected: FRAME_LEN,
        })?;
        Ok(Self(arr))
    }

    /// Parse a 64-byte feature report buffer (byte 0 = report ID 2, dropped).
    pub fn from_feature_report(buf: &[u8]) -> Result<Self, ProtocolError> {
        if buf.len() != FEATURE_REPORT_LEN {
            return Err(ProtocolError::FrameLength {
                got: buf.len(),
                expected: FEATURE_REPORT_LEN,
            });
        }
        if buf[0] != REPORT_ID {
            return Err(ProtocolError::Value {
                field: "report id",
                value: buf[0],
            });
        }
        Self::from_bytes(&buf[1..])
    }

    /// 64-byte buffer for a feature-report write: report ID 2 + payload.
    pub fn to_feature_report(&self) -> [u8; FEATURE_REPORT_LEN] {
        let mut out = [0u8; FEATURE_REPORT_LEN];
        out[0] = REPORT_ID;
        out[1..].copy_from_slice(&self.0);
        out
    }

    pub fn as_bytes(&self) -> &[u8; FRAME_LEN] {
        &self.0
    }

    pub fn status(&self) -> Status {
        Status::from_byte(self.0[STATUS])
    }

    /// Full transaction byte (counter + target bits).
    pub fn transaction_id(&self) -> u8 {
        self.0[TRANSACTION]
    }

    pub fn target(&self) -> Target {
        Target::from_bits(self.0[TRANSACTION] >> TARGET_SHIFT)
    }

    pub fn counter(&self) -> u8 {
        self.0[TRANSACTION] & COUNTER_MASK
    }

    /// Low byte of the big-endian size field (Synapse reads only this byte).
    pub fn data_size(&self) -> u8 {
        self.0[DATA_SIZE_LO]
    }

    /// The full 16-bit big-endian size field (offsets 4..6).
    pub fn data_size16(&self) -> u16 {
        u16::from_be_bytes([self.0[DATA_SIZE_HI], self.0[DATA_SIZE_LO]])
    }

    pub fn class(&self) -> u8 {
        self.0[CLASS]
    }

    pub fn command_id(&self) -> u8 {
        self.0[COMMAND_ID]
    }

    /// Full 53-byte argument area.
    pub fn args(&self) -> &[u8] {
        &self.0[ARGS..ARGS + ARGS_LEN]
    }

    pub fn checksum(&self) -> u8 {
        self.0[CHECKSUM]
    }

    /// Synapse does not check reply checksums (observed); exposed for diagnostics.
    pub fn checksum_valid(&self) -> bool {
        checksum(&self.0) == self.0[CHECKSUM]
    }

    /// True if `self` is the reply to `request`: same transaction byte, class and
    /// command ID (Synapse's observed validation).
    pub fn answers(&self, request: &Frame) -> bool {
        self.transaction_id() == request.transaction_id()
            && self.class() == request.class()
            && self.command_id() == request.command_id()
    }

    /// The same frame in the 2.5 [`Report`] shape, so the typed parsers
    /// (`parse_*(report.args())`) and the session's retry rules work unchanged.
    /// Status, transaction byte, data size, class and id are copied, the 53
    /// argument bytes fill the start of the 80-byte area (rest zero), and the
    /// 2.5 CRC is recomputed: [`Report::crc`] of the result is not the 4.0
    /// wire checksum.
    pub fn to_report(&self) -> Report {
        let mut b = [0u8; report::REPORT_LEN];
        b[0] = self.0[STATUS];
        b[1] = self.0[TRANSACTION];
        b[5] = self.0[DATA_SIZE_LO];
        b[6] = self.0[CLASS];
        b[7] = self.0[COMMAND_ID];
        b[8..8 + ARGS_LEN].copy_from_slice(self.args());
        b[88] = report::crc(&b);
        Report::from_bytes(&b).expect("90-byte buffer")
    }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Frame40 {{ status: {:?}, tid: {:#04x}, cmd: {:02x}/{:02x}, size: {}, args: ",
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
        write!(f, ", checksum: {:#04x} }}", self.checksum())
    }
}

/// XOR of bytes 2..=60 (observed: the 2.5 XOR rule, run before bytes 61/62 are set).
pub fn checksum(frame: &[u8; FRAME_LEN]) -> u8 {
    frame[2..CHECKSUM].iter().fold(0, |acc, b| acc ^ b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Command;

    const FW: Command = Command::new(0x00, 0x81, 2);

    #[test]
    fn encode_get_firmware_exact_bytes() {
        let mut c = TransactionCounter::default();
        let tid = c.next_id(Target::Mouse);
        assert_eq!(tid, 0x01);
        let f = Frame::encode(&Request::new(FW, &[]).unwrap(), tid).unwrap();
        let mut want = [0u8; FEATURE_REPORT_LEN];
        want[..8].copy_from_slice(&[0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00]);
        want[8] = 0x81;
        // checksum = 0x02 ^ 0x00 ^ 0x81 = 0x83 at payload offset 61 (wire offset 62)
        want[62] = 0x83;
        assert_eq!(f.to_feature_report(), want);
        assert!(f.checksum_valid());
    }

    #[test]
    fn encode_set_dpi_exact_bytes() {
        // 2.5 "set DPI": class 0x04 id 0x05, 7 bytes: storage, X BE16, Y BE16, 0, 0.
        let cmd = Command::new(0x04, 0x05, 7);
        let req = Request::new(cmd, &[0x01, 0x03, 0x20, 0x03, 0x20, 0, 0]).unwrap();
        let f = Frame::encode(&req, 0x05).unwrap();
        let w = f.to_feature_report();
        assert_eq!(
            &w[..16],
            &[
                0x02, 0x00, 0x05, 0x00, 0x00, 0x00, 0x07, 0x04, 0x05, 0x01, 0x03, 0x20, 0x03, 0x20,
                0x00, 0x00
            ]
        );
        assert!(w[16..62].iter().all(|&b| b == 0));
        // 07 ^ 04 ^ 05 ^ 01 ^ 03 ^ 20 ^ 03 ^ 20 = 0x07
        assert_eq!(w[62], 0x07);
        assert_eq!(w[63], 0x00);
    }

    #[test]
    fn checksum_excludes_status_and_transaction() {
        let f = Frame::encode(&Request::new(FW, &[]).unwrap(), 1).unwrap();
        let mut b = *f.as_bytes();
        let c = checksum(&b);
        b[0] = 0x02;
        b[1] = 0xFF;
        assert_eq!(checksum(&b), c);
    }

    #[test]
    fn oversized_request_rejected() {
        let cmd = Command::new(0x0F, 0x03, 54);
        let req = Request::new(cmd, &[]).unwrap();
        assert_eq!(
            Frame::encode(&req, 1),
            Err(ProtocolError::FrameDataSize { size: 54, max: 53 })
        );
        let ok = Request::with_size(cmd, 53, &[0xAA; 53]).unwrap();
        let f = Frame::encode(&ok, 1).unwrap();
        assert_eq!(f.args(), &[0xAA; 53]);
    }

    #[test]
    fn counter_runs_1_to_31_with_target_bits() {
        let mut c = TransactionCounter::default();
        let ids: Vec<u8> = (0..33).map(|_| c.next_id(Target::Mouse)).collect();
        assert_eq!(ids[0], 1);
        assert_eq!(ids[30], 31);
        assert_eq!(ids[31], 1);
        assert!(ids.iter().all(|&i| (1..=31).contains(&i)));
        let mut c = TransactionCounter::default();
        assert_eq!(c.next_id(Target::DockDongle), 0xE1);
        assert_eq!(c.next_id(Target::Keyboard), 0x82);
        assert_eq!(c.next_id(Target::Audio), 0x63);
    }

    #[test]
    fn target_roundtrip() {
        for t in 0..8u8 {
            assert_eq!(Target::from_bits(t) as u8, t);
        }
        let f = Frame::encode(&Request::new(FW, &[]).unwrap(), 0xE5).unwrap();
        assert_eq!(f.target(), Target::DockDongle);
        assert_eq!(f.counter(), 5);
    }

    #[test]
    fn feature_report_roundtrip_and_report_id_check() {
        let f = Frame::encode(&Request::new(FW, &[]).unwrap(), 3).unwrap();
        let wire = f.to_feature_report();
        assert_eq!(Frame::from_feature_report(&wire).unwrap(), f);
        let mut wrong = wire;
        wrong[0] = 0;
        assert!(Frame::from_feature_report(&wrong).is_err());
        assert!(Frame::from_feature_report(&wire[..63]).is_err());
    }

    #[test]
    fn reply_matching_and_normalised_report() {
        let req = Frame::encode(&Request::new(FW, &[]).unwrap(), 0x07).unwrap();
        let mut b = *req.as_bytes();
        b[0] = 0x02; // success
        b[8] = 1; // firmware 1.4
        b[9] = 4;
        let reply = Frame::from_bytes(&b).unwrap();
        assert!(reply.answers(&req));
        assert_eq!(reply.status(), Status::Success);
        let r = reply.to_report();
        assert_eq!(r.status(), Status::Success);
        assert_eq!(r.transaction_id(), 0x07);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x00, 0x81, 2));
        assert_eq!(&r.args()[..2], &[1, 4]);
        assert!(r.args()[ARGS_LEN..].iter().all(|&x| x == 0));
        assert!(r.answers(&req.to_report()));
        let other = Frame::encode(&Request::new(FW, &[]).unwrap(), 0x08).unwrap();
        assert!(!reply.answers(&other));
    }

    #[test]
    fn notification_status_is_broadcast() {
        // 0x0A marks an unsolicited notification in 4.0 (observed); the 2.5 table
        // names the same value "Broadcast".
        assert_eq!(Status::from_byte(0x0A), Status::Broadcast);
    }
}
