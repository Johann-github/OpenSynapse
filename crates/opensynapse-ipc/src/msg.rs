//! IPC message types. Pure data; the wire encoding lives in [`crate::codec`].
//!
//! Every connection starts with [`ClientFrame::Hello`] / [`ServerFrame::Welcome`]
//! and has one [`Role`] for its whole life: `Control` (strict request → response)
//! or `Subscribe` (the server only writes [`ServerFrame::Event`]s).

use opensynapse_protocol::buttons::ButtonAssignment;
use opensynapse_protocol::device::{DeviceMode, FirmwareVersion};
use opensynapse_protocol::dpi::{self, Dpi, DpiStages};
use opensynapse_protocol::power::{BatteryLevel, ChargingStatus};

/// Wire protocol version, exchanged in the handshake.
pub const PROTOCOL_VERSION: u16 = 1;

/// What a connection is for, fixed in the handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Control,
    Subscribe,
}

/// Client → agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientFrame {
    Hello { version: u16, role: Role },
    Request { id: u32, req: Request },
}

/// Agent → client.
// Frames are short-lived (one per encode/decode); boxing the snapshot reply would
// only add an allocation and noise at every match.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerFrame {
    Welcome {
        version: u16,
        agent_pid: u32,
    },
    Response {
        id: u32,
        result: Result<Reply, IpcError>,
    },
    Event(Event),
}

/// Which device a request targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceRef {
    /// The only connected device with this USB product ID.
    Pid(u16),
    /// The control collection's path, as reported in [`DeviceInfo::path`].
    Path(String),
}

/// Write options; same meaning as the GUI write options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WireWriteOpts {
    /// Razer software runs and the user confirmed writing anyway.
    pub confirmed: bool,
    /// The user allowed editing left/right click.
    pub allow_primary: bool,
    /// Writing back a value just read from the device.
    pub restore: bool,
}

/// Which polling-rate command pair the device answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollingVariant {
    Period,
    Hyper,
}

/// One writable setting with its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireSetting {
    /// Active DPI, volatile storage.
    Dpi(Dpi),
    Polling {
        hz: u32,
        variant: PollingVariant,
    },
    /// Sleep timer in seconds.
    Sleep(u16),
    /// One button slot (profile, button, mode) with its action.
    Button(ButtonAssignment),
    DpiStages {
        storage: dpi::Storage,
        stages: DpiStages,
    },
}

/// A value read from the device, or why it could not be read.
pub type Field<T> = Result<T, String>;

/// Everything the panel shows, read in one go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireSnapshot {
    pub firmware: Field<FirmwareVersion>,
    pub serial: Field<String>,
    pub mode: Field<(DeviceMode, u8)>,
    pub battery: Field<BatteryLevel>,
    pub charging: Field<ChargingStatus>,
    pub dpi: Field<Dpi>,
    pub stages: Field<DpiStages>,
    pub polling: Field<(u32, PollingVariant)>,
    pub sleep: Field<u16>,
    pub buttons_normal: Field<Vec<ButtonAssignment>>,
    pub buttons_hypershift: Field<Vec<ButtonAssignment>>,
}

/// One connected device (its control collection).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub path: String,
    pub pid: u16,
    /// Registry name, if the device is known.
    pub name: Option<String>,
    /// Protocol family from the registry (e.g. `2.5`).
    pub protocol: String,
    /// The registry knows the device (writes are allowed at all).
    pub supported: bool,
}

/// Control requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Ping,
    ListDevices,
    ReadSnapshot {
        device: DeviceRef,
    },
    Write {
        device: DeviceRef,
        setting: WireSetting,
        opts: WireWriteOpts,
    },
    /// Slice 2.
    ApplyProfile {
        device: DeviceRef,
        toml: String,
        dry_run: bool,
        all: bool,
        opts: WireWriteOpts,
    },
    Shutdown,
}

/// A finished write: the value before, the value asked for, the value read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteDone {
    pub old: WireSetting,
    pub new: WireSetting,
    pub now: WireSetting,
    pub verified: bool,
}

/// A write that did not finish; `old` is set when the old value was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteFailed {
    pub old: Option<WireSetting>,
    pub message: String,
}

/// Successful answers to [`Request`]s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Pong {
        agent_version: String,
    },
    Devices {
        list: Vec<DeviceInfo>,
        /// Running Razer software that may talk to the devices too.
        conflict: Option<String>,
    },
    Snapshot(WireSnapshot),
    Written(Result<WriteDone, WriteFailed>),
    /// Slice 2.
    ProfileApplied {
        lines: Vec<String>,
    },
    ShuttingDown,
}

/// Why a request was not served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    BadRequest,
    Version,
    NoSuchDevice,
    Ambiguous,
    Unsupported,
    Conflict,
    Device,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcError {
    pub kind: ErrorKind,
    pub message: String,
}

/// Pushed to `Subscribe` connections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    DeviceArrived(DeviceInfo),
    DeviceRemoved {
        path: String,
        pid: u16,
    },
    Battery {
        path: String,
        level: u8,
        charging: u8,
    },
    DpiChanged {
        path: String,
        x: u16,
        y: u16,
    },
    Hardware {
        path: String,
        id: u8,
        payload: Vec<u8>,
    },
    Error {
        message: String,
    },
    ShuttingDown,
}
