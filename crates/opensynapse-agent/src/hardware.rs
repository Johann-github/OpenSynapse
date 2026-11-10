//! What the core needs from the machine: enumerate, open, conflict check.
//! [`RealHardware`] goes through `opensynapse-hid`; the tests use a fake mouse.

use opensynapse_hid::{
    DEFAULT_INTERFACE, DeviceSession, FeatureIo, HidEntry, Session, Timing, Transport,
};

/// Device access as the core sees it.
pub trait Hardware {
    type Io: FeatureIo;

    /// All Razer HID collections present now (control and sibling collections).
    fn enumerate(&mut self) -> Result<Vec<HidEntry>, String>;

    /// Open a session on a control collection from the last [`Self::enumerate`].
    fn open(&mut self, entry: &HidEntry) -> Result<Session<Self::Io>, String>;

    /// Running Razer software that may talk to the devices at the same time.
    fn conflict(&self) -> Option<String>;
}

/// Names the I/O type behind the public `DeviceSession` alias.
pub trait SessionIo {
    type Io;
}

impl<D> SessionIo for Session<D> {
    type Io = D;
}

/// The control collection of a device, as the GUI and CLI pick it.
pub fn is_control(e: &HidEntry) -> bool {
    e.is_control(DEFAULT_INTERFACE)
}

/// Real HID devices through `opensynapse-hid`.
#[derive(Default)]
pub struct RealHardware {
    transport: Option<Transport>,
}

impl Hardware for RealHardware {
    type Io = <DeviceSession as SessionIo>::Io;

    fn enumerate(&mut self) -> Result<Vec<HidEntry>, String> {
        let t = Transport::new().map_err(|e| format!("cannot enumerate HID devices: {e}"))?;
        let entries = t.entries().to_vec();
        self.transport = Some(t);
        Ok(entries)
    }

    fn open(&mut self, entry: &HidEntry) -> Result<Session<Self::Io>, String> {
        if self.transport.is_none() {
            self.enumerate()?;
        }
        let t = self.transport.as_ref().expect("set above");
        t.open(entry, Timing::default())
            .map_err(|e| format!("cannot open device: {e}"))
    }

    fn conflict(&self) -> Option<String> {
        opensynapse_hid::conflicting_process().map(str::to_owned)
    }
}

/// In-memory devices for the core tests.
#[cfg(test)]
pub mod fake {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use opensynapse_device::fake::FakeMouse;
    use opensynapse_hid::{CONTROL_USAGE, CONTROL_USAGE_PAGE, HidEntry, Session, Timing};

    use super::Hardware;

    /// Viper V3 HyperSpeed product ID (in the registry, protocol 2.5).
    pub const VIPER_PID: u16 = 0x00B8;

    /// Plugged-in fake mice by path; tests add and remove them.
    #[derive(Default)]
    pub struct FakeHardware {
        pub mice: BTreeMap<String, (u16, FakeMouse)>,
        pub conflict: Option<String>,
        /// Number of successful `open` calls.
        pub opens: usize,
    }

    impl FakeHardware {
        pub fn plug(&mut self, path: &str, pid: u16) -> FakeMouse {
            let m = FakeMouse::new();
            self.mice.insert(path.into(), (pid, m.clone()));
            m
        }
    }

    pub fn control_entry(path: &str, pid: u16) -> HidEntry {
        HidEntry {
            vid: 0x1532,
            pid,
            interface: 0,
            usage_page: CONTROL_USAGE_PAGE,
            usage: CONTROL_USAGE,
            feature_len: Some(91),
            path: path.into(),
        }
    }

    fn fast() -> Timing {
        Timing {
            between_out: Duration::ZERO,
            out_to_in: Duration::ZERO,
            between_in: Duration::ZERO,
            max_in: 2,
            max_out: 2,
        }
    }

    impl Hardware for FakeHardware {
        type Io = FakeMouse;

        fn enumerate(&mut self) -> Result<Vec<HidEntry>, String> {
            Ok(self
                .mice
                .iter()
                .map(|(p, (pid, _))| control_entry(p, *pid))
                .collect())
        }

        fn open(&mut self, entry: &HidEntry) -> Result<Session<FakeMouse>, String> {
            let (_, m) = self
                .mice
                .get(&entry.path)
                .ok_or_else(|| "cannot open device: gone".to_string())?;
            self.opens += 1;
            Ok(Session::new(m.clone(), fast()))
        }

        fn conflict(&self) -> Option<String> {
            self.conflict.clone()
        }
    }
}
