//! A fake mouse that answers feature reports from in-memory state (Viper V3
//! HyperSpeed layout). Reply argument layouts are produced with the protocol's own
//! `set_*` builders (a GET reply has the same layout as the matching SET request).
//!
//! Clones share one state, so a test can keep a handle while a session (or another
//! thread) owns the device.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use opensynapse_hid::{FeatureIo, HidError};
use opensynapse_protocol::command::{self, Command};
use opensynapse_protocol::{FEATURE_REPORT_LEN, Report, Request, dpi, polling, power};

pub use crate::VIPER_BUTTONS;
use crate::{HYPER_PROFILE, stages_from};

/// The fake's factory stage list: 400 [800].
pub fn default_stages() -> dpi::DpiStages {
    stages_from(&[400, 800], 1).expect("valid default stages")
}

/// What the fake mouse holds and how it behaves.
#[derive(Clone, Debug)]
pub struct State {
    pub dpi: dpi::Dpi,
    pub polling_hz: u32,
    pub sleep: u16,
    pub period_supported: bool,
    /// Accept writes but keep the old value (read-back mismatch).
    pub ignore_writes: bool,
    pub answers: bool,
    /// Answer every report with status Timeout (0x04), like a sleeping
    /// wireless mouse behind its dongle (observed 2026-11-10).
    pub asleep: bool,
    /// (mode, button) -> (function, data); Viper defaults: button N -> mouse
    /// button N, DPI button -> DPI function.
    pub slots: BTreeMap<(u8, u8), (u8, Vec<u8>)>,
    /// Stage lists by storage byte.
    pub stages: [dpi::DpiStages; 2],
    /// Profile byte of every button assignment GET/SET (02/8C, 02/0C) answered.
    pub button_profiles: Vec<u8>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            dpi: dpi::Dpi::uniform(800),
            polling_hz: 1000,
            sleep: 300,
            period_supported: true,
            ignore_writes: false,
            answers: true,
            asleep: false,
            slots: [0u8, 1]
                .into_iter()
                .flat_map(|m| {
                    VIPER_BUTTONS.map(|b| {
                        let v = if b == 96 { (6, vec![6]) } else { (1, vec![b]) };
                        ((m, b), v)
                    })
                })
                .collect(),
            stages: [default_stages(), default_stages()],
            button_profiles: Vec::new(),
        }
    }
}

#[derive(Default)]
struct Inner {
    state: State,
    pending: Option<[u8; FEATURE_REPORT_LEN]>,
    /// (class, id) of every report sent.
    sent: Vec<(u8, u8)>,
}

/// In-memory mouse; `Clone` shares the same state.
#[derive(Clone, Default)]
pub struct FakeMouse {
    inner: Arc<Mutex<Inner>>,
}

impl FakeMouse {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The mouse's state, to inspect or change between exchanges.
    pub fn state(&self) -> MappedState<'_> {
        MappedState(self.lock())
    }

    /// (class, id) of every report sent so far.
    pub fn sent(&self) -> Vec<(u8, u8)> {
        self.lock().sent.clone()
    }

    /// Number of SET reports (command ID without the 0x80 GET bit) sent so far.
    pub fn writes(&self) -> usize {
        self.lock()
            .sent
            .iter()
            .filter(|(_, id)| id & 0x80 == 0)
            .count()
    }
}

/// Locked view of a [`FakeMouse`]'s [`State`].
pub struct MappedState<'a>(MutexGuard<'a, Inner>);

impl std::ops::Deref for MappedState<'_> {
    type Target = State;
    fn deref(&self) -> &State {
        &self.0.state
    }
}

impl std::ops::DerefMut for MappedState<'_> {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.0.state
    }
}

/// Args of an encoded request (to borrow its layout for a reply).
fn args_of(r: Request) -> Vec<u8> {
    r.encode(0).args().to_vec()
}

/// Reply = the request echoed with `status` and `args` in the argument area.
fn reply(request: &[u8], status: u8, args: &[u8]) -> [u8; FEATURE_REPORT_LEN] {
    let mut out = [0u8; FEATURE_REPORT_LEN];
    out.copy_from_slice(request);
    out[1] = status;
    out[9..9 + args.len()].copy_from_slice(args);
    out
}

/// Answer one request from `st`: (status byte, reply args).
fn answer(st: &mut State, req: &Report) -> (u8, Vec<u8>) {
    let is = |c: Command| c.class == req.class() && c.id == req.command_id();
    let ok = |args: Vec<u8>| (2u8, args);
    if st.asleep {
        (4, vec![])
    } else if !st.answers {
        (5, vec![])
    } else if is(command::GET_FIRMWARE_VERSION) {
        ok(vec![1, 4])
    } else if is(command::GET_SERIAL) {
        ok(b"PM-TEST".to_vec())
    } else if is(command::GET_DEVICE_MODE) {
        ok(vec![0, 0])
    } else if is(command::GET_BATTERY_LEVEL) {
        ok(vec![0, 255])
    } else if is(command::GET_CHARGING_STATUS) {
        ok(vec![0, 1])
    } else if is(command::GET_DPI) {
        ok(args_of(dpi::set_dpi(dpi::Storage::Volatile, st.dpi)))
    } else if is(command::SET_DPI) {
        if !st.ignore_writes {
            st.dpi = dpi::parse_dpi(req.args()).unwrap();
        }
        ok(vec![])
    } else if is(command::GET_DPI_STAGES) {
        let storage = req.args()[0];
        let st_val = if storage == 1 {
            dpi::Storage::Persistent
        } else {
            dpi::Storage::Volatile
        };
        let stages = st.stages[usize::from(storage)].clone();
        ok(args_of(dpi::set_dpi_stages(st_val, &stages).unwrap()))
    } else if is(command::SET_DPI_STAGES) {
        if !st.ignore_writes {
            let new = dpi::parse_dpi_stages(req.args()).unwrap();
            // Setting the active stage also applies its DPI (observed).
            st.dpi = new.stages[new.active_index().unwrap()].dpi;
            st.stages[usize::from(req.args()[0])] = new;
        }
        ok(vec![])
    } else if is(command::GET_POLLING_PERIOD) {
        if st.period_supported {
            ok(args_of(polling::set_polling_rate(st.polling_hz).unwrap()))
        } else {
            (5, vec![])
        }
    } else if is(command::SET_POLLING_PERIOD) {
        if !st.ignore_writes {
            st.polling_hz = polling::parse_polling_rate(req.args()).unwrap();
        }
        ok(vec![])
    } else if is(command::GET_HYPER_POLLING) {
        ok(args_of(
            polling::set_hyper_polling(HYPER_PROFILE, st.polling_hz).unwrap(),
        ))
    } else if is(command::GET_SLEEP_TIME) {
        ok(args_of(power::set_sleep_time(st.sleep)))
    } else if is(command::SET_SLEEP_TIME) {
        if !st.ignore_writes {
            st.sleep = power::parse_sleep_time(req.args()).unwrap();
        }
        ok(vec![])
    } else if is(command::GET_BUTTON_ASSIGNMENT) {
        let (p, b, m) = (req.args()[0], req.args()[1], req.args()[2]);
        st.button_profiles.push(p);
        match st.slots.get(&(m, b)) {
            Some((function, data)) => {
                // Reply byte 2 is not the mode (observed); the fake sends 1.
                let mut args = vec![p, b, 1, *function, data.len() as u8];
                args.extend_from_slice(data);
                ok(args)
            }
            None => (5, vec![]),
        }
    } else if is(command::SET_BUTTON_ASSIGNMENT) {
        let a = req.args();
        let (b, m, len) = (a[1], a[2], usize::from(a[4]));
        st.button_profiles.push(a[0]);
        if !st.ignore_writes && st.slots.contains_key(&(m, b)) {
            st.slots.insert((m, b), (a[3], a[5..5 + len].to_vec()));
        }
        ok(vec![])
    } else if is(command::GET_BUTTON_ASSIGNMENT_LIST) {
        let (p, m) = (req.args()[0], req.args()[1]);
        let mut args = vec![p, m, 0];
        for b in VIPER_BUTTONS {
            let (function, data) = &st.slots[&(m, b)];
            let mut rec = [0u8; 8];
            rec[..3].copy_from_slice(&[b, *function, data.len() as u8]);
            rec[3..3 + data.len()].copy_from_slice(data);
            args.extend_from_slice(&rec);
        }
        ok(args)
    } else {
        (5, vec![])
    }
}

impl FeatureIo for FakeMouse {
    fn send_feature(&self, buf: &[u8]) -> Result<(), HidError> {
        let req = Report::from_feature_report(buf)?;
        let mut inner = self.lock();
        inner.sent.push((req.class(), req.command_id()));
        let (status, args) = answer(&mut inner.state, &req);
        inner.pending = Some(reply(buf, status, &args));
        Ok(())
    }

    fn get_feature(&self, buf: &mut [u8]) -> Result<usize, HidError> {
        let pending = self.lock().pending.expect("GET before SET");
        buf.copy_from_slice(&pending);
        Ok(FEATURE_REPORT_LEN)
    }
}
