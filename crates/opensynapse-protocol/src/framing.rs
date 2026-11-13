//! Wire framing per protocol family: one entry point that turns a [`Request`]
//! into feature-report bytes and a reply buffer back into a [`Report`].
//!
//! Requests and replies are always handled in the 2.5 [`Report`] shape; only the
//! wire bytes differ. Owner of "which frame goes on the wire": this module.
//! Decision: `vault/Decisions/ADR-0015 Protocol 4.0 and 3.0 support.md`.

use crate::report::{self, Report, TransactionCounter};
use crate::report40::{self, Frame, Target};
use crate::{ProtocolError, Request};

/// The frame format a device speaks on its control collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Framing {
    /// Protocol 2.5: report ID 0, 90-byte payload (hardware-tested on the Viper V3 HyperSpeed).
    #[default]
    V25,
    /// Protocol 4.0: report ID 2, 63-byte payload, 2.5 command tables (inferred, untested).
    V40(Target),
}

/// Largest feature report of any framing (2.5: 91 bytes).
pub const MAX_FEATURE_REPORT_LEN: usize = report::FEATURE_REPORT_LEN;

/// Feature-report bytes, report ID first. Fixed-size storage, no allocation.
#[derive(Clone, PartialEq, Eq)]
pub struct Wire {
    buf: [u8; MAX_FEATURE_REPORT_LEN],
    len: usize,
}

impl Wire {
    fn from_slice(bytes: &[u8]) -> Self {
        let mut buf = [0u8; MAX_FEATURE_REPORT_LEN];
        buf[..bytes.len()].copy_from_slice(bytes);
        Self {
            buf,
            len: bytes.len(),
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf[..self.len]
    }
}

impl std::fmt::Debug for Wire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Wire({} bytes)", self.len)
    }
}

impl Framing {
    /// HID report ID (byte 0 of every feature report).
    pub fn report_id(self) -> u8 {
        match self {
            Self::V25 => 0,
            Self::V40(_) => report40::REPORT_ID,
        }
    }

    /// Feature report length including the report-ID byte.
    pub fn feature_len(self) -> usize {
        match self {
            Self::V25 => report::FEATURE_REPORT_LEN,
            Self::V40(_) => report40::FEATURE_REPORT_LEN,
        }
    }

    /// Zeroed buffer for a feature-report read, report ID in byte 0.
    pub fn reply_buffer(self) -> Wire {
        let mut w = Wire::from_slice(&[0u8; MAX_FEATURE_REPORT_LEN][..self.feature_len()]);
        w.buf[0] = self.report_id();
        w
    }

    /// Parse a feature-report buffer read from the device into the 2.5 shape.
    pub fn decode(self, buf: &[u8]) -> Result<Report, ProtocolError> {
        match self {
            Self::V25 => Report::from_feature_report(buf),
            Self::V40(_) => Ok(Frame::from_feature_report(buf)?.to_report()),
        }
    }
}

/// Owns the transaction counter of one device session and encodes requests.
#[derive(Clone, Debug, Default)]
pub struct Encoder {
    framing: Framing,
    counter25: TransactionCounter,
    counter40: report40::TransactionCounter,
}

impl Encoder {
    pub fn new(framing: Framing) -> Self {
        Self {
            framing,
            ..Self::default()
        }
    }

    pub fn framing(&self) -> Framing {
        self.framing
    }

    /// Next transaction: the request in 2.5 shape (to match replies with
    /// [`Report::answers`]) and the bytes to send.
    pub fn encode(&mut self, request: &Request) -> Result<(Report, Wire), ProtocolError> {
        match self.framing {
            Framing::V25 => {
                let report = request.encode(self.counter25.next_id());
                let wire = Wire::from_slice(&report.to_feature_report());
                Ok((report, wire))
            }
            Framing::V40(target) => {
                let frame = Frame::encode(request, self.counter40.next_id(target))?;
                Ok((
                    frame.to_report(),
                    Wire::from_slice(&frame.to_feature_report()),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::get_firmware_version;

    #[test]
    fn v25_bytes_unchanged() {
        // The 2.5 path must produce exactly what Request::encode produced before.
        let mut e = Encoder::new(Framing::V25);
        let (r0, w0) = e.encode(&get_firmware_version()).unwrap();
        let (_, w1) = e.encode(&get_firmware_version()).unwrap();
        assert_eq!(
            w0.as_slice(),
            &get_firmware_version().encode(0).to_feature_report()[..]
        );
        assert_eq!(
            w1.as_slice(),
            &get_firmware_version().encode(1).to_feature_report()[..]
        );
        assert_eq!(r0.transaction_id(), 0);
        assert_eq!(Framing::V25.decode(w0.as_slice()).unwrap(), r0);
    }

    #[test]
    fn v40_wire_and_decode() {
        let mut e = Encoder::new(Framing::V40(Target::Mouse));
        let (req, wire) = e.encode(&get_firmware_version()).unwrap();
        assert_eq!(wire.as_slice().len(), 64);
        assert_eq!(&wire.as_slice()[..9], &[2, 0, 1, 0, 0, 0, 2, 0, 0x81]);
        assert_eq!(wire.as_slice()[62], 0x83);
        let mut reply = wire.clone();
        reply.as_mut_slice()[1] = 0x02; // status success
        let r = Framing::V40(Target::Mouse)
            .decode(reply.as_slice())
            .unwrap();
        assert!(r.answers(&req));
        // 2.5-length buffers are rejected by the 4.0 decoder and vice versa.
        assert!(Framing::V40(Target::Mouse).decode(&[0u8; 91]).is_err());
        assert!(Framing::V25.decode(wire.as_slice()).is_err());
    }

    #[test]
    fn reply_buffers() {
        let b = Framing::V25.reply_buffer();
        assert_eq!(b.as_slice(), &[0u8; 91][..]);
        let b = Framing::V40(Target::Mouse).reply_buffer();
        assert_eq!(b.as_slice().len(), 64);
        assert_eq!(b.as_slice()[0], 2);
    }
}
