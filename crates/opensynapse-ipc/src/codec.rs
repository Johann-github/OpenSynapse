//! Versioned binary encoding of [`crate::msg`] frames. Pure, std only.
//!
//! Integers little-endian; `bool` strictly 0/1; strings and lists carry a `u32`
//! length; `Option` tag 0/1; `Result` 0 = Ok / 1 = Err; every enum variant has an
//! explicit, never-reused tag. The decoder must consume the whole body.
//!
//! Tags are wire constants: a removed variant keeps its number reserved, a new
//! variant takes a new number. Changing a layout means bumping
//! [`crate::msg::PROTOCOL_VERSION`].

use opensynapse_protocol::buttons::{ButtonAssignment, Mode};
use opensynapse_protocol::device::{DeviceMode, FirmwareVersion};
use opensynapse_protocol::dpi::{Dpi, DpiStage, DpiStages, Storage};
use opensynapse_protocol::power::{BatteryLevel, ChargingStatus};

use crate::msg::{
    ClientFrame, DeviceInfo, DeviceRef, ErrorKind, Event, IpcError, PollingVariant, Reply, Request,
    Role, ServerFrame, WireSetting, WireSnapshot, WireWriteOpts, WriteDone, WriteFailed,
};

/// Why a frame body could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("message ends early")]
    Truncated,
    #[error("unknown tag {tag} for {what}")]
    UnknownTag { what: &'static str, tag: u8 },
    #[error("invalid bool byte {0}")]
    InvalidBool(u8),
    #[error("string is not valid UTF-8")]
    InvalidUtf8,
    #[error("length {0} exceeds the remaining bytes")]
    Length(u32),
    #[error("{0} trailing bytes after the message")]
    Trailing(usize),
    #[error("invalid value: {0}")]
    Invalid(String),
}

pub fn encode_client(f: &ClientFrame) -> Vec<u8> {
    let mut w = Writer::default();
    w.client_frame(f);
    w.0
}

pub fn decode_client(b: &[u8]) -> Result<ClientFrame, DecodeError> {
    let mut r = Reader::new(b);
    let f = r.client_frame()?;
    r.finish()?;
    Ok(f)
}

pub fn encode_server(f: &ServerFrame) -> Vec<u8> {
    let mut w = Writer::default();
    w.server_frame(f);
    w.0
}

pub fn decode_server(b: &[u8]) -> Result<ServerFrame, DecodeError> {
    let mut r = Reader::new(b);
    let f = r.server_frame()?;
    r.finish()?;
    Ok(f)
}

// ---- tags (wire constants, never reuse a number) ----

mod tag {
    // ClientFrame
    pub const HELLO: u8 = 0;
    pub const REQUEST: u8 = 1;
    // ServerFrame
    pub const WELCOME: u8 = 0;
    pub const RESPONSE: u8 = 1;
    pub const EVENT: u8 = 2;
    // Role
    pub const ROLE_CONTROL: u8 = 0;
    pub const ROLE_SUBSCRIBE: u8 = 1;
    // Request
    pub const REQ_PING: u8 = 0;
    pub const REQ_LIST_DEVICES: u8 = 1;
    pub const REQ_READ_SNAPSHOT: u8 = 2;
    pub const REQ_WRITE: u8 = 3;
    pub const REQ_APPLY_PROFILE: u8 = 4;
    pub const REQ_SHUTDOWN: u8 = 5;
    // Reply
    pub const REP_PONG: u8 = 0;
    pub const REP_DEVICES: u8 = 1;
    pub const REP_SNAPSHOT: u8 = 2;
    pub const REP_WRITTEN: u8 = 3;
    pub const REP_PROFILE_APPLIED: u8 = 4;
    pub const REP_SHUTTING_DOWN: u8 = 5;
    // ErrorKind
    pub const ERR_BAD_REQUEST: u8 = 0;
    pub const ERR_VERSION: u8 = 1;
    pub const ERR_NO_SUCH_DEVICE: u8 = 2;
    pub const ERR_AMBIGUOUS: u8 = 3;
    pub const ERR_UNSUPPORTED: u8 = 4;
    pub const ERR_CONFLICT: u8 = 5;
    pub const ERR_DEVICE: u8 = 6;
    // Event
    pub const EV_DEVICE_ARRIVED: u8 = 0;
    pub const EV_DEVICE_REMOVED: u8 = 1;
    pub const EV_BATTERY: u8 = 2;
    pub const EV_DPI_CHANGED: u8 = 3;
    pub const EV_HARDWARE: u8 = 4;
    pub const EV_ERROR: u8 = 5;
    pub const EV_SHUTTING_DOWN: u8 = 6;
    // DeviceRef
    pub const DEV_PID: u8 = 0;
    pub const DEV_PATH: u8 = 1;
    // PollingVariant
    pub const POLL_PERIOD: u8 = 0;
    pub const POLL_HYPER: u8 = 1;
    // WireSetting
    pub const SET_DPI: u8 = 0;
    pub const SET_POLLING: u8 = 1;
    pub const SET_SLEEP: u8 = 2;
    pub const SET_BUTTON: u8 = 3;
    pub const SET_DPI_STAGES: u8 = 4;
    // dpi::Storage
    pub const STORE_VOLATILE: u8 = 0;
    pub const STORE_PERSISTENT: u8 = 1;
    // buttons::Mode
    pub const MODE_NORMAL: u8 = 0;
    pub const MODE_HYPERSHIFT: u8 = 1;
    // DeviceMode
    pub const DM_NORMAL: u8 = 0;
    pub const DM_BOOTLOADER: u8 = 1;
    pub const DM_TESTING: u8 = 2;
    pub const DM_DRIVER: u8 = 3;
    pub const DM_DOWNLOAD: u8 = 4;
    pub const DM_PAIRING: u8 = 5;
    pub const DM_STANDBY: u8 = 6;
    pub const DM_RF_TEST: u8 = 7;
    pub const DM_RETAIL: u8 = 8;
    pub const DM_OTHER: u8 = 9;
    // ChargingStatus
    pub const CS_NOT_CHARGING: u8 = 0;
    pub const CS_CHARGING: u8 = 1;
    pub const CS_FAILED: u8 = 2;
    pub const CS_FOREIGN_OBJECT: u8 = 3;
    pub const CS_FAST_CHARGING: u8 = 4;
    pub const CS_FULL: u8 = 5;
    pub const CS_LIMIT_REACHED: u8 = 6;
    pub const CS_OTHER: u8 = 7;
    // Option / Result
    pub const NONE: u8 = 0;
    pub const SOME: u8 = 1;
    pub const OK: u8 = 0;
    pub const ERR: u8 = 1;
}

// ---- writer ----

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn len(&mut self, n: usize) {
        // Frames are capped at MAX_FRAME (1 MiB), far below u32::MAX.
        self.u32(u32::try_from(n).expect("IPC length fits in u32"));
    }
    fn bytes(&mut self, b: &[u8]) {
        self.len(b.len());
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    fn opt<T>(&mut self, v: &Option<T>, f: impl FnOnce(&mut Self, &T)) {
        match v {
            None => self.u8(tag::NONE),
            Some(x) => {
                self.u8(tag::SOME);
                f(self, x);
            }
        }
    }
    fn result<T, E>(
        &mut self,
        v: &Result<T, E>,
        ok: impl FnOnce(&mut Self, &T),
        err: impl FnOnce(&mut Self, &E),
    ) {
        match v {
            Ok(x) => {
                self.u8(tag::OK);
                ok(self, x);
            }
            Err(e) => {
                self.u8(tag::ERR);
                err(self, e);
            }
        }
    }
    fn field<T>(&mut self, v: &Result<T, String>, ok: impl FnOnce(&mut Self, &T)) {
        self.result(v, ok, |w, e| w.str(e));
    }
    fn list<T>(&mut self, v: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.len(v.len());
        for x in v {
            f(self, x);
        }
    }

    fn client_frame(&mut self, f: &ClientFrame) {
        match f {
            ClientFrame::Hello { version, role } => {
                self.u8(tag::HELLO);
                self.u16(*version);
                self.role(*role);
            }
            ClientFrame::Request { id, req } => {
                self.u8(tag::REQUEST);
                self.u32(*id);
                self.request(req);
            }
        }
    }

    fn server_frame(&mut self, f: &ServerFrame) {
        match f {
            ServerFrame::Welcome { version, agent_pid } => {
                self.u8(tag::WELCOME);
                self.u16(*version);
                self.u32(*agent_pid);
            }
            ServerFrame::Response { id, result } => {
                self.u8(tag::RESPONSE);
                self.u32(*id);
                self.result(result, Self::reply, Self::ipc_error);
            }
            ServerFrame::Event(e) => {
                self.u8(tag::EVENT);
                self.event(e);
            }
        }
    }

    fn role(&mut self, r: Role) {
        self.u8(match r {
            Role::Control => tag::ROLE_CONTROL,
            Role::Subscribe => tag::ROLE_SUBSCRIBE,
        });
    }

    fn request(&mut self, r: &Request) {
        match r {
            Request::Ping => self.u8(tag::REQ_PING),
            Request::ListDevices => self.u8(tag::REQ_LIST_DEVICES),
            Request::ReadSnapshot { device } => {
                self.u8(tag::REQ_READ_SNAPSHOT);
                self.device_ref(device);
            }
            Request::Write {
                device,
                setting,
                opts,
            } => {
                self.u8(tag::REQ_WRITE);
                self.device_ref(device);
                self.setting(setting);
                self.write_opts(opts);
            }
            Request::ApplyProfile {
                device,
                toml,
                dry_run,
                all,
                opts,
            } => {
                self.u8(tag::REQ_APPLY_PROFILE);
                self.device_ref(device);
                self.str(toml);
                self.bool(*dry_run);
                self.bool(*all);
                self.write_opts(opts);
            }
            Request::Shutdown => self.u8(tag::REQ_SHUTDOWN),
        }
    }

    fn reply(&mut self, r: &Reply) {
        match r {
            Reply::Pong { agent_version } => {
                self.u8(tag::REP_PONG);
                self.str(agent_version);
            }
            Reply::Devices { list, conflict } => {
                self.u8(tag::REP_DEVICES);
                self.list(list, Self::device_info);
                self.opt(conflict, |w, s| w.str(s));
            }
            Reply::Snapshot(s) => {
                self.u8(tag::REP_SNAPSHOT);
                self.snapshot(s);
            }
            Reply::Written(res) => {
                self.u8(tag::REP_WRITTEN);
                self.result(res, Self::write_done, Self::write_failed);
            }
            Reply::ProfileApplied {
                lines,
                restore_toml,
                error,
            } => {
                self.u8(tag::REP_PROFILE_APPLIED);
                self.list(lines, |w, s| w.str(s));
                self.opt(restore_toml, |w, s| w.str(s));
                self.opt(error, |w, s| w.str(s));
            }
            Reply::ShuttingDown => self.u8(tag::REP_SHUTTING_DOWN),
        }
    }

    fn ipc_error(&mut self, e: &IpcError) {
        self.u8(match e.kind {
            ErrorKind::BadRequest => tag::ERR_BAD_REQUEST,
            ErrorKind::Version => tag::ERR_VERSION,
            ErrorKind::NoSuchDevice => tag::ERR_NO_SUCH_DEVICE,
            ErrorKind::Ambiguous => tag::ERR_AMBIGUOUS,
            ErrorKind::Unsupported => tag::ERR_UNSUPPORTED,
            ErrorKind::Conflict => tag::ERR_CONFLICT,
            ErrorKind::Device => tag::ERR_DEVICE,
        });
        self.str(&e.message);
    }

    fn event(&mut self, e: &Event) {
        match e {
            Event::DeviceArrived(info) => {
                self.u8(tag::EV_DEVICE_ARRIVED);
                self.device_info(info);
            }
            Event::DeviceRemoved { path, pid } => {
                self.u8(tag::EV_DEVICE_REMOVED);
                self.str(path);
                self.u16(*pid);
            }
            Event::Battery {
                path,
                level,
                charging,
            } => {
                self.u8(tag::EV_BATTERY);
                self.str(path);
                self.u8(*level);
                self.u8(*charging);
            }
            Event::DpiChanged { path, x, y } => {
                self.u8(tag::EV_DPI_CHANGED);
                self.str(path);
                self.u16(*x);
                self.u16(*y);
            }
            Event::Hardware { path, id, payload } => {
                self.u8(tag::EV_HARDWARE);
                self.str(path);
                self.u8(*id);
                self.bytes(payload);
            }
            Event::Error { message } => {
                self.u8(tag::EV_ERROR);
                self.str(message);
            }
            Event::ShuttingDown => self.u8(tag::EV_SHUTTING_DOWN),
        }
    }

    fn device_ref(&mut self, d: &DeviceRef) {
        match d {
            DeviceRef::Pid(pid) => {
                self.u8(tag::DEV_PID);
                self.u16(*pid);
            }
            DeviceRef::Path(p) => {
                self.u8(tag::DEV_PATH);
                self.str(p);
            }
        }
    }

    fn device_info(&mut self, d: &DeviceInfo) {
        self.str(&d.path);
        self.u16(d.pid);
        self.opt(&d.name, |w, s| w.str(s));
        self.str(&d.protocol);
        self.bool(d.supported);
    }

    fn write_opts(&mut self, o: &WireWriteOpts) {
        self.bool(o.confirmed);
        self.bool(o.allow_primary);
        self.bool(o.restore);
    }

    fn polling_variant(&mut self, v: PollingVariant) {
        self.u8(match v {
            PollingVariant::Period => tag::POLL_PERIOD,
            PollingVariant::Hyper => tag::POLL_HYPER,
        });
    }

    fn setting(&mut self, s: &WireSetting) {
        match s {
            WireSetting::Dpi(d) => {
                self.u8(tag::SET_DPI);
                self.dpi(d);
            }
            WireSetting::Polling { hz, variant } => {
                self.u8(tag::SET_POLLING);
                self.u32(*hz);
                self.polling_variant(*variant);
            }
            WireSetting::Sleep(secs) => {
                self.u8(tag::SET_SLEEP);
                self.u16(*secs);
            }
            WireSetting::Button(b) => {
                self.u8(tag::SET_BUTTON);
                self.button(b);
            }
            WireSetting::DpiStages { storage, stages } => {
                self.u8(tag::SET_DPI_STAGES);
                self.storage(*storage);
                self.stages(stages);
            }
        }
    }

    fn write_done(&mut self, d: &WriteDone) {
        self.setting(&d.old);
        self.setting(&d.new);
        self.setting(&d.now);
        self.bool(d.verified);
    }

    fn write_failed(&mut self, f: &WriteFailed) {
        self.opt(&f.old, Self::setting);
        self.str(&f.message);
    }

    fn snapshot(&mut self, s: &WireSnapshot) {
        self.field(&s.firmware, |w, v| {
            w.u8(v.major);
            w.u8(v.minor);
        });
        self.field(&s.serial, |w, v| w.str(v));
        self.field(&s.mode, |w, (m, b)| {
            w.device_mode(*m);
            w.u8(*b);
        });
        self.field(&s.battery, |w, v| w.u8(v.0));
        self.field(&s.charging, |w, v| w.charging(*v));
        self.field(&s.dpi, Self::dpi);
        self.field(&s.stages, Self::stages);
        self.field(&s.polling, |w, (hz, v)| {
            w.u32(*hz);
            w.polling_variant(*v);
        });
        self.field(&s.sleep, |w, v| w.u16(*v));
        self.field(&s.buttons_normal, |w, v| w.list(v, Self::button));
        self.field(&s.buttons_hypershift, |w, v| w.list(v, Self::button));
    }

    fn dpi(&mut self, d: &Dpi) {
        self.u16(d.x);
        self.u16(d.y);
    }

    fn stages(&mut self, s: &DpiStages) {
        self.u8(s.active);
        self.list(&s.stages, |w, st| {
            w.u8(st.id);
            w.dpi(&st.dpi);
        });
    }

    fn storage(&mut self, s: Storage) {
        self.u8(match s {
            Storage::Volatile => tag::STORE_VOLATILE,
            Storage::Persistent => tag::STORE_PERSISTENT,
        });
    }

    fn button(&mut self, b: &ButtonAssignment) {
        self.u8(b.profile);
        self.u8(b.button);
        self.u8(match b.mode {
            Mode::Normal => tag::MODE_NORMAL,
            Mode::HyperShift => tag::MODE_HYPERSHIFT,
        });
        self.u8(b.function);
        self.bytes(&b.data);
    }

    fn device_mode(&mut self, m: DeviceMode) {
        match m {
            DeviceMode::Normal => self.u8(tag::DM_NORMAL),
            DeviceMode::Bootloader => self.u8(tag::DM_BOOTLOADER),
            DeviceMode::Testing => self.u8(tag::DM_TESTING),
            DeviceMode::Driver => self.u8(tag::DM_DRIVER),
            DeviceMode::Download => self.u8(tag::DM_DOWNLOAD),
            DeviceMode::Pairing => self.u8(tag::DM_PAIRING),
            DeviceMode::Standby => self.u8(tag::DM_STANDBY),
            DeviceMode::RfTest => self.u8(tag::DM_RF_TEST),
            DeviceMode::Retail => self.u8(tag::DM_RETAIL),
            DeviceMode::Other(b) => {
                self.u8(tag::DM_OTHER);
                self.u8(b);
            }
        }
    }

    fn charging(&mut self, c: ChargingStatus) {
        match c {
            ChargingStatus::NotCharging => self.u8(tag::CS_NOT_CHARGING),
            ChargingStatus::Charging => self.u8(tag::CS_CHARGING),
            ChargingStatus::Failed => self.u8(tag::CS_FAILED),
            ChargingStatus::ForeignObject => self.u8(tag::CS_FOREIGN_OBJECT),
            ChargingStatus::FastCharging => self.u8(tag::CS_FAST_CHARGING),
            ChargingStatus::Full => self.u8(tag::CS_FULL),
            ChargingStatus::LimitReached => self.u8(tag::CS_LIMIT_REACHED),
            ChargingStatus::Other(b) => {
                self.u8(tag::CS_OTHER);
                self.u8(b);
            }
        }
    }
}

// ---- reader ----

struct Reader<'a> {
    buf: &'a [u8],
}

type R<T> = Result<T, DecodeError>;

fn unknown<T>(what: &'static str, tag: u8) -> R<T> {
    Err(DecodeError::UnknownTag { what, tag })
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    fn finish(&self) -> R<()> {
        match self.buf.len() {
            0 => Ok(()),
            n => Err(DecodeError::Trailing(n)),
        }
    }

    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        if self.buf.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Ok(head)
    }

    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> R<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> R<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn bool(&mut self) -> R<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(DecodeError::InvalidBool(b)),
        }
    }
    /// A length or element count; never more than the bytes left, so a bogus
    /// count cannot make the decoder allocate or loop beyond the frame.
    fn len(&mut self) -> R<usize> {
        let n = self.u32()?;
        let len = usize::try_from(n).map_err(|_| DecodeError::Length(n))?;
        if len > self.buf.len() {
            return Err(DecodeError::Length(n));
        }
        Ok(len)
    }
    fn bytes(&mut self) -> R<Vec<u8>> {
        let n = self.len()?;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> R<String> {
        let n = self.len()?;
        let b = self.take(n)?;
        std::str::from_utf8(b)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8)
    }
    fn opt<T>(&mut self, f: impl FnOnce(&mut Self) -> R<T>) -> R<Option<T>> {
        match self.u8()? {
            tag::NONE => Ok(None),
            tag::SOME => Ok(Some(f(self)?)),
            t => unknown("Option", t),
        }
    }
    fn result<T, E>(
        &mut self,
        ok: impl FnOnce(&mut Self) -> R<T>,
        err: impl FnOnce(&mut Self) -> R<E>,
    ) -> R<Result<T, E>> {
        match self.u8()? {
            tag::OK => Ok(Ok(ok(self)?)),
            tag::ERR => Ok(Err(err(self)?)),
            t => unknown("Result", t),
        }
    }
    fn field<T>(&mut self, ok: impl FnOnce(&mut Self) -> R<T>) -> R<Result<T, String>> {
        self.result(ok, Self::str)
    }
    fn list<T>(&mut self, mut f: impl FnMut(&mut Self) -> R<T>) -> R<Vec<T>> {
        let n = self.len()?;
        // No pre-allocation: `n` is bounded by the bytes left, not by memory.
        let mut v = Vec::new();
        for _ in 0..n {
            v.push(f(self)?);
        }
        Ok(v)
    }

    fn client_frame(&mut self) -> R<ClientFrame> {
        match self.u8()? {
            tag::HELLO => Ok(ClientFrame::Hello {
                version: self.u16()?,
                role: self.role()?,
            }),
            tag::REQUEST => Ok(ClientFrame::Request {
                id: self.u32()?,
                req: self.request()?,
            }),
            t => unknown("ClientFrame", t),
        }
    }

    fn server_frame(&mut self) -> R<ServerFrame> {
        match self.u8()? {
            tag::WELCOME => Ok(ServerFrame::Welcome {
                version: self.u16()?,
                agent_pid: self.u32()?,
            }),
            tag::RESPONSE => Ok(ServerFrame::Response {
                id: self.u32()?,
                result: self.result(Self::reply, Self::ipc_error)?,
            }),
            tag::EVENT => Ok(ServerFrame::Event(self.event()?)),
            t => unknown("ServerFrame", t),
        }
    }

    fn role(&mut self) -> R<Role> {
        match self.u8()? {
            tag::ROLE_CONTROL => Ok(Role::Control),
            tag::ROLE_SUBSCRIBE => Ok(Role::Subscribe),
            t => unknown("Role", t),
        }
    }

    fn request(&mut self) -> R<Request> {
        match self.u8()? {
            tag::REQ_PING => Ok(Request::Ping),
            tag::REQ_LIST_DEVICES => Ok(Request::ListDevices),
            tag::REQ_READ_SNAPSHOT => Ok(Request::ReadSnapshot {
                device: self.device_ref()?,
            }),
            tag::REQ_WRITE => Ok(Request::Write {
                device: self.device_ref()?,
                setting: self.setting()?,
                opts: self.write_opts()?,
            }),
            tag::REQ_APPLY_PROFILE => Ok(Request::ApplyProfile {
                device: self.device_ref()?,
                toml: self.str()?,
                dry_run: self.bool()?,
                all: self.bool()?,
                opts: self.write_opts()?,
            }),
            tag::REQ_SHUTDOWN => Ok(Request::Shutdown),
            t => unknown("Request", t),
        }
    }

    fn reply(&mut self) -> R<Reply> {
        match self.u8()? {
            tag::REP_PONG => Ok(Reply::Pong {
                agent_version: self.str()?,
            }),
            tag::REP_DEVICES => Ok(Reply::Devices {
                list: self.list(Self::device_info)?,
                conflict: self.opt(Self::str)?,
            }),
            tag::REP_SNAPSHOT => Ok(Reply::Snapshot(self.snapshot()?)),
            tag::REP_WRITTEN => Ok(Reply::Written(
                self.result(Self::write_done, Self::write_failed)?,
            )),
            tag::REP_PROFILE_APPLIED => Ok(Reply::ProfileApplied {
                lines: self.list(Self::str)?,
                restore_toml: self.opt(Self::str)?,
                error: self.opt(Self::str)?,
            }),
            tag::REP_SHUTTING_DOWN => Ok(Reply::ShuttingDown),
            t => unknown("Reply", t),
        }
    }

    fn ipc_error(&mut self) -> R<IpcError> {
        let kind = match self.u8()? {
            tag::ERR_BAD_REQUEST => ErrorKind::BadRequest,
            tag::ERR_VERSION => ErrorKind::Version,
            tag::ERR_NO_SUCH_DEVICE => ErrorKind::NoSuchDevice,
            tag::ERR_AMBIGUOUS => ErrorKind::Ambiguous,
            tag::ERR_UNSUPPORTED => ErrorKind::Unsupported,
            tag::ERR_CONFLICT => ErrorKind::Conflict,
            tag::ERR_DEVICE => ErrorKind::Device,
            t => return unknown("ErrorKind", t),
        };
        Ok(IpcError {
            kind,
            message: self.str()?,
        })
    }

    fn event(&mut self) -> R<Event> {
        match self.u8()? {
            tag::EV_DEVICE_ARRIVED => Ok(Event::DeviceArrived(self.device_info()?)),
            tag::EV_DEVICE_REMOVED => Ok(Event::DeviceRemoved {
                path: self.str()?,
                pid: self.u16()?,
            }),
            tag::EV_BATTERY => Ok(Event::Battery {
                path: self.str()?,
                level: self.u8()?,
                charging: self.u8()?,
            }),
            tag::EV_DPI_CHANGED => Ok(Event::DpiChanged {
                path: self.str()?,
                x: self.u16()?,
                y: self.u16()?,
            }),
            tag::EV_HARDWARE => Ok(Event::Hardware {
                path: self.str()?,
                id: self.u8()?,
                payload: self.bytes()?,
            }),
            tag::EV_ERROR => Ok(Event::Error {
                message: self.str()?,
            }),
            tag::EV_SHUTTING_DOWN => Ok(Event::ShuttingDown),
            t => unknown("Event", t),
        }
    }

    fn device_ref(&mut self) -> R<DeviceRef> {
        match self.u8()? {
            tag::DEV_PID => Ok(DeviceRef::Pid(self.u16()?)),
            tag::DEV_PATH => Ok(DeviceRef::Path(self.str()?)),
            t => unknown("DeviceRef", t),
        }
    }

    fn device_info(&mut self) -> R<DeviceInfo> {
        Ok(DeviceInfo {
            path: self.str()?,
            pid: self.u16()?,
            name: self.opt(Self::str)?,
            protocol: self.str()?,
            supported: self.bool()?,
        })
    }

    fn write_opts(&mut self) -> R<WireWriteOpts> {
        Ok(WireWriteOpts {
            confirmed: self.bool()?,
            allow_primary: self.bool()?,
            restore: self.bool()?,
        })
    }

    fn polling_variant(&mut self) -> R<PollingVariant> {
        match self.u8()? {
            tag::POLL_PERIOD => Ok(PollingVariant::Period),
            tag::POLL_HYPER => Ok(PollingVariant::Hyper),
            t => unknown("PollingVariant", t),
        }
    }

    fn setting(&mut self) -> R<WireSetting> {
        match self.u8()? {
            tag::SET_DPI => Ok(WireSetting::Dpi(self.dpi()?)),
            tag::SET_POLLING => Ok(WireSetting::Polling {
                hz: self.u32()?,
                variant: self.polling_variant()?,
            }),
            tag::SET_SLEEP => Ok(WireSetting::Sleep(self.u16()?)),
            tag::SET_BUTTON => Ok(WireSetting::Button(self.button()?)),
            tag::SET_DPI_STAGES => Ok(WireSetting::DpiStages {
                storage: self.storage()?,
                stages: self.stages()?,
            }),
            t => unknown("WireSetting", t),
        }
    }

    fn write_done(&mut self) -> R<WriteDone> {
        Ok(WriteDone {
            old: self.setting()?,
            new: self.setting()?,
            now: self.setting()?,
            verified: self.bool()?,
        })
    }

    fn write_failed(&mut self) -> R<WriteFailed> {
        Ok(WriteFailed {
            old: self.opt(Self::setting)?,
            message: self.str()?,
        })
    }

    fn snapshot(&mut self) -> R<WireSnapshot> {
        Ok(WireSnapshot {
            firmware: self.field(|r| {
                Ok(FirmwareVersion {
                    major: r.u8()?,
                    minor: r.u8()?,
                })
            })?,
            serial: self.field(Self::str)?,
            mode: self.field(|r| Ok((r.device_mode()?, r.u8()?)))?,
            battery: self.field(|r| Ok(BatteryLevel(r.u8()?)))?,
            charging: self.field(Self::charging)?,
            dpi: self.field(Self::dpi)?,
            stages: self.field(Self::stages)?,
            polling: self.field(|r| Ok((r.u32()?, r.polling_variant()?)))?,
            sleep: self.field(Self::u16)?,
            buttons_normal: self.field(|r| r.list(Self::button))?,
            buttons_hypershift: self.field(|r| r.list(Self::button))?,
        })
    }

    fn dpi(&mut self) -> R<Dpi> {
        Ok(Dpi {
            x: self.u16()?,
            y: self.u16()?,
        })
    }

    fn stages(&mut self) -> R<DpiStages> {
        Ok(DpiStages {
            active: self.u8()?,
            stages: self.list(|r| {
                Ok(DpiStage {
                    id: r.u8()?,
                    dpi: r.dpi()?,
                })
            })?,
        })
    }

    fn storage(&mut self) -> R<Storage> {
        match self.u8()? {
            tag::STORE_VOLATILE => Ok(Storage::Volatile),
            tag::STORE_PERSISTENT => Ok(Storage::Persistent),
            t => unknown("Storage", t),
        }
    }

    fn button(&mut self) -> R<ButtonAssignment> {
        Ok(ButtonAssignment {
            profile: self.u8()?,
            button: self.u8()?,
            mode: match self.u8()? {
                tag::MODE_NORMAL => Mode::Normal,
                tag::MODE_HYPERSHIFT => Mode::HyperShift,
                t => return unknown("ButtonMode", t),
            },
            function: self.u8()?,
            data: self.bytes()?,
        })
    }

    fn device_mode(&mut self) -> R<DeviceMode> {
        match self.u8()? {
            tag::DM_NORMAL => Ok(DeviceMode::Normal),
            tag::DM_BOOTLOADER => Ok(DeviceMode::Bootloader),
            tag::DM_TESTING => Ok(DeviceMode::Testing),
            tag::DM_DRIVER => Ok(DeviceMode::Driver),
            tag::DM_DOWNLOAD => Ok(DeviceMode::Download),
            tag::DM_PAIRING => Ok(DeviceMode::Pairing),
            tag::DM_STANDBY => Ok(DeviceMode::Standby),
            tag::DM_RF_TEST => Ok(DeviceMode::RfTest),
            tag::DM_RETAIL => Ok(DeviceMode::Retail),
            tag::DM_OTHER => Ok(DeviceMode::Other(self.u8()?)),
            t => unknown("DeviceMode", t),
        }
    }

    fn charging(&mut self) -> R<ChargingStatus> {
        match self.u8()? {
            tag::CS_NOT_CHARGING => Ok(ChargingStatus::NotCharging),
            tag::CS_CHARGING => Ok(ChargingStatus::Charging),
            tag::CS_FAILED => Ok(ChargingStatus::Failed),
            tag::CS_FOREIGN_OBJECT => Ok(ChargingStatus::ForeignObject),
            tag::CS_FAST_CHARGING => Ok(ChargingStatus::FastCharging),
            tag::CS_FULL => Ok(ChargingStatus::Full),
            tag::CS_LIMIT_REACHED => Ok(ChargingStatus::LimitReached),
            tag::CS_OTHER => Ok(ChargingStatus::Other(self.u8()?)),
            t => unknown("ChargingStatus", t),
        }
    }
}
