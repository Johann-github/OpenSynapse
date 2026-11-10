//! Pluggable hardware-event sources. Slice 1 has none ([`NoEvents`]): which
//! collection the Viper sends its notifications on is still unknown
//! (`vault/Protocol/Hardware Events.md`).

use std::sync::mpsc::Sender;

use opensynapse_hid::HidEntry;
use opensynapse_protocol::event;

use crate::convert;
use crate::core::CoreMsg;

/// Where an event source delivers raw input reports of one device.
// Slice 1 ships only `NoEvents`, so nothing pushes yet outside the tests.
#[cfg_attr(not(test), allow(dead_code))]
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
    #[cfg_attr(not(test), allow(dead_code))]
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

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
}
