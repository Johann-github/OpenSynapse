use super::*;
use opensynapse_protocol::device::get_firmware_version;
use std::cell::RefCell;
use std::collections::VecDeque;

/// What the fake device answers to one GET.
#[derive(Clone, Copy)]
enum Reply {
    /// Echo the last request with this status and args `[1, 2]`.
    Status(u8),
    /// Echo with a wrong transaction ID.
    WrongTid,
    Short,
}

#[derive(Default)]
struct Mock {
    sent: RefCell<Vec<Vec<u8>>>,
    replies: RefCell<VecDeque<Reply>>,
}

impl Mock {
    fn with(replies: &[Reply]) -> Self {
        Self {
            replies: RefCell::new(replies.iter().copied().collect()),
            ..Default::default()
        }
    }
}

impl FeatureIo for &Mock {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError> {
        assert_eq!(buf.len(), 91);
        assert_eq!(buf[0], 0, "report ID");
        self.sent.borrow_mut().push(buf.to_vec());
        Ok(())
    }

    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError> {
        let last = self.sent.borrow().last().cloned().expect("GET before SET");
        let reply = self
            .replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected GET");
        buf.copy_from_slice(&last);
        match reply {
            Reply::Status(s) => {
                buf[1] = s;
                buf[9] = 1;
                buf[10] = 2;
            }
            Reply::WrongTid => {
                buf[1] = 0x02;
                buf[2] ^= 0x10;
            }
            Reply::Short => return Ok(4),
        }
        Ok(91)
    }
}

fn fast() -> Timing {
    Timing {
        between_out: Duration::ZERO,
        out_to_in: Duration::ZERO,
        between_in: Duration::ZERO,
        max_in: 3,
        max_out: 3,
    }
}

#[test]
fn success_first_try() {
    let m = Mock::with(&[Reply::Status(0x02)]);
    let mut s = Session::new(&m, fast());
    let r = s.execute(&get_firmware_version()).unwrap();
    assert_eq!(&r.args()[..2], &[1, 2]);
    assert_eq!(m.sent.borrow().len(), 1);
}

#[test]
fn polls_while_busy() {
    let m = Mock::with(&[
        Reply::Status(0x01),
        Reply::Status(0x01),
        Reply::Status(0x02),
    ]);
    let mut s = Session::new(&m, fast());
    assert!(s.execute(&get_firmware_version()).is_ok());
    assert_eq!(m.sent.borrow().len(), 1);
}

#[test]
fn resends_on_failure_and_mismatch() {
    let m = Mock::with(&[Reply::Status(0x03), Reply::WrongTid, Reply::Status(0x02)]);
    let mut s = Session::new(&m, fast());
    assert!(s.execute(&get_firmware_version()).is_ok());
    let sent = m.sent.borrow();
    assert_eq!(sent.len(), 3);
    // A resend repeats the same report (same transaction ID).
    assert!(sent.iter().all(|r| r == &sent[0]));
}

#[test]
fn not_supported_is_permanent() {
    let m = Mock::with(&[Reply::Status(0x05)]);
    let mut s = Session::new(&m, fast());
    assert!(matches!(
        s.execute(&get_firmware_version()),
        Err(HidError::Status(Status::NotSupported))
    ));
    assert_eq!(m.sent.borrow().len(), 1);
}

#[test]
fn gives_up_after_max_out() {
    let busy = [Reply::Status(0x01); 9];
    let m = Mock::with(&busy);
    let mut s = Session::new(&m, fast());
    let err = s.execute(&get_firmware_version()).unwrap_err();
    assert!(
        matches!(err, HidError::Exhausted { attempts: 3, .. }),
        "{err}"
    );
    assert_eq!(m.sent.borrow().len(), 3);
}

/// A sleeping wireless mouse: the dongle answers status Timeout (0x04) to
/// every send (observed 2026-11-10, Agent slice 1 test).
#[test]
fn timeout_every_time_is_no_response() {
    let m = Mock::with(&[Reply::Status(0x04); 3]);
    let mut s = Session::new(&m, fast());
    let err = s.execute(&get_firmware_version()).unwrap_err();
    assert!(matches!(err, HidError::NoResponse { attempts: 3 }), "{err}");
    assert!(err.is_no_response());
    assert!(is_no_response_text(&format!("cannot read DPI: {err}")));
    assert_eq!(m.sent.borrow().len(), 3);
}

#[test]
fn timeout_then_success_is_ok() {
    let m = Mock::with(&[Reply::Status(0x04), Reply::Status(0x02)]);
    let mut s = Session::new(&m, fast());
    assert!(s.execute(&get_firmware_version()).is_ok());
}

#[test]
fn last_attempt_decides_the_class() {
    // Timeouts first, then a plain failure: not the sleep class.
    let m = Mock::with(&[
        Reply::Status(0x04),
        Reply::Status(0x04),
        Reply::Status(0x03),
    ]);
    let mut s = Session::new(&m, fast());
    let err = s.execute(&get_firmware_version()).unwrap_err();
    assert!(matches!(err, HidError::Exhausted { .. }), "{err}");
    assert!(!err.is_no_response());
    assert!(!is_no_response_text(&err.to_string()));
}

#[test]
fn short_read_resends() {
    let m = Mock::with(&[Reply::Short, Reply::Status(0x02)]);
    let mut s = Session::new(&m, fast());
    assert!(s.execute(&get_firmware_version()).is_ok());
    assert_eq!(m.sent.borrow().len(), 2);
}

#[test]
fn transaction_ids_advance() {
    let m = Mock::with(&[Reply::Status(0x02), Reply::Status(0x02)]);
    let mut s = Session::new(&m, fast());
    s.execute(&get_firmware_version()).unwrap();
    s.execute(&get_firmware_version()).unwrap();
    let sent = m.sent.borrow();
    assert_eq!((sent[0][2], sent[1][2]), (0, 1));
}

#[test]
fn control_entry_filter() {
    let e = HidEntry {
        vid: 0x1532,
        pid: 0x00B8,
        interface: 0,
        usage_page: 1,
        usage: 2,
        feature_len: Some(91),
        input_len: Some(9),
        output_len: Some(0),
        path: "x".into(),
    };
    assert!(e.is_control(0));
    assert!(!e.is_control(1));
    let consumer = HidEntry {
        usage_page: 0x000C,
        usage: 1,
        ..e
    };
    assert!(!consumer.is_control(0));
    let short_feature = HidEntry {
        feature_len: Some(9),
        ..consumer.clone()
    };
    assert!(!short_feature.is_control(0));
}

#[test]
fn v25_wire_bytes_unchanged() {
    // Regression guard for the hardware-tested 2.5 path (Viper V3 HyperSpeed).
    let m = Mock::with(&[Reply::Status(0x02)]);
    let mut s = Session::new(&m, fast());
    assert_eq!(s.framing(), Framing::V25);
    s.execute(&get_firmware_version()).unwrap();
    let want = get_firmware_version().encode(0).to_feature_report();
    assert_eq!(m.sent.borrow()[0], want.to_vec());
}

/// Fake protocol 4.0 device: 64-byte feature reports with report ID 2.
#[derive(Default)]
struct Mock40 {
    sent: RefCell<Vec<Vec<u8>>>,
    statuses: RefCell<VecDeque<u8>>,
}

impl Mock40 {
    fn with(statuses: &[u8]) -> Self {
        Self {
            statuses: RefCell::new(statuses.iter().copied().collect()),
            ..Default::default()
        }
    }
}

impl FeatureIo for &Mock40 {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError> {
        assert_eq!(buf.len(), 64);
        assert_eq!(buf[0], 2, "report ID");
        self.sent.borrow_mut().push(buf.to_vec());
        Ok(())
    }

    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError> {
        assert_eq!(buf.len(), 64);
        assert_eq!(buf[0], 2, "report ID on entry");
        let last = self.sent.borrow().last().cloned().expect("GET before SET");
        buf.copy_from_slice(&last);
        buf[1] = self
            .statuses
            .borrow_mut()
            .pop_front()
            .expect("unexpected GET");
        buf[9] = 1;
        buf[10] = 4;
        Ok(64)
    }
}

fn v40() -> Framing {
    Framing::V40(opensynapse_protocol::report40::Target::Mouse)
}

#[test]
fn v40_session_exact_bytes_and_reply() {
    let m = Mock40::with(&[0x01, 0x02]);
    let mut s = Session::with_framing(&m, fast(), v40());
    let r = s.execute(&get_firmware_version()).unwrap();
    assert_eq!(&r.args()[..2], &[1, 4]);
    let sent = m.sent.borrow();
    assert_eq!(sent.len(), 1, "busy is polled, not resent");
    let mut want = vec![0u8; 64];
    want[..9].copy_from_slice(&[0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x81]);
    want[62] = 0x83;
    assert_eq!(sent[0], want);
}

#[test]
fn v40_failure_resends_and_unsupported_is_permanent() {
    let m = Mock40::with(&[0x03, 0x05]);
    let mut s = Session::with_framing(&m, fast(), v40());
    assert!(matches!(
        s.execute(&get_firmware_version()),
        Err(HidError::Status(Status::NotSupported))
    ));
    assert_eq!(m.sent.borrow().len(), 2);
}

#[test]
fn v40_transaction_counter_starts_at_1() {
    let m = Mock40::with(&[0x02, 0x02]);
    let mut s = Session::with_framing(&m, fast(), v40());
    s.execute(&get_firmware_version()).unwrap();
    s.execute(&get_firmware_version()).unwrap();
    let sent = m.sent.borrow();
    assert_eq!((sent[0][2], sent[1][2]), (1, 2));
}

#[test]
fn v40_oversized_request_is_a_protocol_error() {
    let m = Mock40::default();
    let mut s = Session::with_framing(&m, fast(), v40());
    let cmd = opensynapse_protocol::Command::new(0x0F, 0x03, 60);
    let req = Request::new(cmd, &[]).unwrap();
    assert!(matches!(s.execute(&req), Err(HidError::Protocol(_))));
    assert!(m.sent.borrow().is_empty());
}

#[test]
fn v40_control_entry_filter() {
    let e = HidEntry {
        vid: 0x1532,
        pid: 0x0F00,
        interface: 0,
        usage_page: 0xFF00,
        usage: 1,
        feature_len: Some(64),
        input_len: None,
        output_len: None,
        path: "x".into(),
    };
    assert!(e.is_control_for(v40(), 0));
    assert!(!e.is_control_for(v40(), 1));
    assert!(!e.is_control_for(Framing::V25, 0));
    let viper = HidEntry {
        pid: 0x00B8,
        usage_page: 1,
        usage: 2,
        feature_len: Some(91),
        ..e.clone()
    };
    assert!(!viper.is_control_for(v40(), 0));
    assert!(viper.is_control_for(Framing::V25, 0));
    // Unknown length (hidapi): 4.0 falls back to the mouse usage pair.
    let no_len = HidEntry {
        feature_len: None,
        ..viper.clone()
    };
    assert!(no_len.is_control_for(v40(), 0));
    assert!(!HidEntry { usage: 6, ..no_len }.is_control_for(v40(), 0));
}

#[test]
fn registry_picks_framing() {
    assert_eq!(framing_for_pid(0x00B8), Framing::V25);
    // Not in the registry: the 2.5 shape, as before 4.0 support.
    assert_eq!(framing_for_pid(0xFFFF), Framing::V25);
}
