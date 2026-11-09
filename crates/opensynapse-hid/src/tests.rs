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
