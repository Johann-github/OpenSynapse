//! Pluggable hardware-event sources (`vault/Protocol/Hardware Events.md`).
//!
//! - [`NoEvents`]: the default until hardware confirms which collection carries
//!   the notifications.
//! - [`HidInputEvents`] (`--events`): opens the device's candidate sibling
//!   collections for reading and forwards notification records (report ID 5/9).
//!   One reader thread per collection, blocked in the kernel; no polling.

use std::sync::mpsc::Sender;

use opensynapse_hid::HidEntry;
use opensynapse_hid::input::{self, OnReport, ReaderHandle};
use opensynapse_protocol::event;

use crate::convert;
use crate::core::CoreMsg;

/// Where an event source delivers raw input reports of one device.
#[derive(Clone)]
pub struct EventSink {
    tx: Sender<CoreMsg>,
    path: String,
}

impl EventSink {
    pub fn new(tx: Sender<CoreMsg>, path: String) -> Self {
        Self { tx, path }
    }

    /// Decode one input report and hand it to the core. Returns `false` once the
    /// core is gone (the source should stop).
    pub fn push(&self, raw: &[u8]) -> bool {
        let Some(n) = event::decode(raw) else {
            return true;
        };
        let ev = convert::notification_to_wire(&self.path, raw, n);
        self.tx.send(CoreMsg::DeviceEvent(ev)).is_ok()
    }
}

/// Starts listening for a device's hardware events.
pub trait EventSource: Send {
    /// `None` when this source has nothing to listen on for the device.
    fn attach(
        &mut self,
        control: &HidEntry,
        siblings: &[HidEntry],
        sink: EventSink,
    ) -> Option<Box<dyn Detach>>;
}

/// Stops listening (called when the device goes away or the agent stops).
pub trait Detach: Send {
    fn detach(self: Box<Self>);
}

/// Slice 1: no event source.
pub struct NoEvents;

impl EventSource for NoEvents {
    fn attach(&mut self, _: &HidEntry, _: &[HidEntry], _: EventSink) -> Option<Box<dyn Detach>> {
        None
    }
}

/// Starts one input-report reader; [`HidOpener`] for real devices, a fake in tests.
pub trait InputOpener: Send {
    fn open(&mut self, path: &str, on_report: OnReport) -> Result<Box<dyn Detach>, String>;
}

/// Reads through `opensynapse_hid::input`.
pub struct HidOpener;

impl InputOpener for HidOpener {
    fn open(&mut self, path: &str, on_report: OnReport) -> Result<Box<dyn Detach>, String> {
        input::spawn(path, on_report)
            .map(|h| Box::new(Reader(h)) as Box<dyn Detach>)
            .map_err(|e| e.to_string())
    }
}

struct Reader(ReaderHandle);

impl Detach for Reader {
    fn detach(self: Box<Self>) {
        if let Err(e) = self.0.stop() {
            tracing::debug!(error = %e, "input reader had ended");
        }
    }
}

/// Which sibling collections to listen on.
pub type Candidate = fn(&HidEntry) -> bool;

/// Default guess until a capture confirms it: a collection with input reports
/// that is vendor-defined (usage page >= 0xFF00), or generic-desktop usage 0 on an
/// interface other than the mouse's (MI_01 Col05-08 on the Viper V3 HS, observed
/// readable in `vault/Protocol/Windows Transport.md`).
pub fn default_candidate(e: &HidEntry) -> bool {
    e.input_len != Some(0)
        && (e.usage_page >= 0xFF00 || (e.usage_page == 0x0001 && e.usage == 0 && e.interface != 0))
}

/// Byte 0 is a notification record (primary or secondary device).
fn is_notification(raw: &[u8]) -> bool {
    matches!(
        raw.first(),
        Some(&(event::RECORD_NOTIFICATION | event::RECORD_SECONDARY_NOTIFICATION))
    )
}

/// Listens on every candidate sibling collection that opens for reading.
pub struct HidInputEvents<O> {
    opener: O,
    candidate: Candidate,
}

impl<O: InputOpener> HidInputEvents<O> {
    pub fn new(opener: O, candidate: Candidate) -> Self {
        Self { opener, candidate }
    }
}

impl<O: InputOpener> EventSource for HidInputEvents<O> {
    fn attach(
        &mut self,
        control: &HidEntry,
        siblings: &[HidEntry],
        sink: EventSink,
    ) -> Option<Box<dyn Detach>> {
        let mut readers = Vec::new();
        for s in siblings.iter().filter(|s| (self.candidate)(s)) {
            let sink = sink.clone();
            // Other records (mouse movement, keys) are dropped here; `false` ends
            // the reader once the core is gone.
            let on_report: OnReport = Box::new(move |raw| !is_notification(raw) || sink.push(raw));
            match self.opener.open(&s.path, on_report) {
                Ok(d) => {
                    tracing::info!(
                        path = %s.path,
                        usage_page = s.usage_page,
                        usage = s.usage,
                        "listening for hardware events"
                    );
                    readers.push(d);
                }
                Err(e) => tracing::debug!(path = %s.path, error = %e, "not readable"),
            }
        }
        if readers.is_empty() {
            tracing::info!(path = %control.path, "no readable event collection");
            return None;
        }
        Some(Box::new(Readers(readers)))
    }
}

struct Readers(Vec<Box<dyn Detach>>);

impl Detach for Readers {
    fn detach(self: Box<Self>) {
        for r in self.0 {
            r.detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};

    use opensynapse_ipc::msg::Event;

    use super::*;

    #[test]
    fn sink_decodes_and_forwards() {
        let (tx, rx) = channel();
        let sink = EventSink::new(tx, "p".into());
        // Not a notification record: ignored.
        assert!(sink.push(&[2, 0xE9, 0]));
        assert!(rx.try_recv().is_err());
        assert!(sink.push(&[5, 0x02, 0x03, 0x20, 0x03, 0x20]));
        let Ok(CoreMsg::DeviceEvent(ev)) = rx.try_recv() else {
            panic!("expected a device event");
        };
        assert_eq!(
            ev,
            Event::DpiChanged {
                path: "p".into(),
                x: 800,
                y: 800
            }
        );
        drop(rx);
        assert!(!sink.push(&[5, 0x05]), "core gone");
    }

    fn entry(path: &str, interface: i32, up: u16, u: u16, input: u16) -> HidEntry {
        HidEntry {
            vid: 0x1532,
            pid: 0x00B8,
            interface,
            usage_page: up,
            usage: u,
            feature_len: Some(0),
            input_len: Some(input),
            output_len: Some(0),
            path: path.into(),
        }
    }

    #[test]
    fn default_candidates() {
        assert!(default_candidate(&entry("v", 1, 0xFF00, 1, 64)));
        assert!(default_candidate(&entry("g", 1, 0x0001, 0, 16)));
        assert!(!default_candidate(&entry("g0", 0, 0x0001, 0, 16)));
        assert!(!default_candidate(&entry("mouse", 0, 0x0001, 2, 9)));
        assert!(!default_candidate(&entry("consumer", 1, 0x000C, 1, 3)));
        assert!(!default_candidate(&entry("no-input", 1, 0xFF00, 1, 0)));
    }

    type Opened = Arc<Mutex<Vec<(String, OnReport)>>>;

    /// Records opened paths and their callbacks; refuses paths starting with "x".
    #[derive(Default)]
    struct FakeOpener {
        opened: Opened,
        detached: Arc<Mutex<Vec<String>>>,
    }

    struct FakeReader(String, Arc<Mutex<Vec<String>>>);

    impl Detach for FakeReader {
        fn detach(self: Box<Self>) {
            self.1.lock().unwrap().push(self.0);
        }
    }

    impl InputOpener for FakeOpener {
        fn open(&mut self, path: &str, f: OnReport) -> Result<Box<dyn Detach>, String> {
            if path.starts_with('x') {
                return Err("access denied".into());
            }
            self.opened.lock().unwrap().push((path.into(), f));
            Ok(Box::new(FakeReader(path.into(), self.detached.clone())))
        }
    }

    #[test]
    fn opens_candidates_and_forwards_notifications() {
        let opener = FakeOpener::default();
        let (opened, detached) = (opener.opened.clone(), opener.detached.clone());
        let mut src = HidInputEvents::new(opener, default_candidate);
        let control = entry("ctl", 0, 1, 2, 9);
        let siblings = [
            entry("vendor", 2, 0xFF00, 1, 64),
            entry("xdenied", 1, 0xFF01, 1, 64),
            entry("gen", 1, 0x0001, 0, 16),
            entry("consumer", 1, 0x000C, 1, 3),
        ];
        let (tx, rx) = channel();
        let d = src
            .attach(&control, &siblings, EventSink::new(tx, "ctl".into()))
            .expect("readers attached");
        let paths: Vec<String> = opened.lock().unwrap().iter().map(|o| o.0.clone()).collect();
        assert_eq!(paths, ["vendor", "gen"]);

        {
            let mut o = opened.lock().unwrap();
            // Mouse movement on the same collection: dropped, reader keeps going.
            assert!((o[0].1)(&[1, 0, 4, 0]));
            assert!(rx.try_recv().is_err());
            assert!((o[1].1)(&[5, 0x02, 0x03, 0x20, 0x03, 0x20]));
        }
        let Ok(CoreMsg::DeviceEvent(ev)) = rx.try_recv() else {
            panic!("expected a device event");
        };
        assert_eq!(
            ev,
            Event::DpiChanged {
                path: "ctl".into(),
                x: 800,
                y: 800
            }
        );

        d.detach();
        assert_eq!(*detached.lock().unwrap(), ["vendor", "gen"]);

        // Core gone: the callback asks the reader to stop.
        drop(rx);
        assert!(!(opened.lock().unwrap()[0].1)(&[5, 0x05]));
    }

    #[test]
    fn nothing_readable_means_no_detacher() {
        let mut src = HidInputEvents::new(FakeOpener::default(), default_candidate);
        let (tx, _rx) = channel();
        let siblings = [
            entry("xvendor", 1, 0xFF00, 1, 64),
            entry("mouse", 0, 1, 2, 9),
        ];
        assert!(
            src.attach(
                &entry("ctl", 0, 1, 2, 9),
                &siblings,
                EventSink::new(tx, "c".into())
            )
            .is_none()
        );
    }
}
