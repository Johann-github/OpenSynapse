//! IPC message types. Pure data; the wire encoding lives in [`crate::codec`].
//!
//! Every connection starts with [`ClientFrame::Hello`] / [`ServerFrame::Welcome`]
//! and has one [`Role`] for its whole life: `Control` (strict request → response)
//! or `Subscribe` (the server only writes [`ServerFrame::Event`]s).

use opensynapse_protocol::buttons::ButtonAssignment;
use opensynapse_protocol::device::{DeviceMode, FirmwareVersion};
use opensynapse_protocol::dpi::{self, Dpi, DpiStages};
use opensynapse_protocol::lighting::Effect;
use opensynapse_protocol::power::{BatteryLevel, ChargingStatus};

/// Wire protocol version, exchanged in the handshake.
/// 2: per-app profiles (`ReloadAppProfiles`, `AppProfiles`, `Event::AppProfile`).
/// 3: `ErrorKind::NoResponse` (device did not answer, e.g. asleep).
/// 4: `WireSetting::Lighting` (ADR-0014).
pub const PROTOCOL_VERSION: u16 = 4;

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
    /// Extended-family lighting of one zone: raw effect (byte 0 = storage, `region` =
    /// zone) and optional raw brightness 0-255.
    Lighting {
        effect: Effect,
        brightness: Option<u8>,
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
    /// Re-read `app-profiles.toml` and re-evaluate the foreground application.
    /// Answered with [`Reply::AppProfiles`] (a parse error is its `error`).
    ReloadAppProfiles,
    /// Show the loaded per-app rules and the active profile.
    AppProfiles,
}

/// One per-app rule, as loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRuleInfo {
    pub exe: String,
    pub profile: String,
}

/// Per-app profile state of the agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppProfilesInfo {
    /// The rules file the agent reads (`None`: no location known).
    pub config: Option<String>,
    /// The rules are loaded (the feature is on).
    pub enabled: bool,
    /// Why the file was not loaded (invalid); `None` when loaded or missing.
    pub error: Option<String>,
    pub default: Option<String>,
    pub rules: Vec<AppRuleInfo>,
    /// Last foreground executable the agent saw.
    pub foreground: Option<String>,
    /// Profile file currently applied by app switching.
    pub active: Option<String>,
    /// Values from before the first app profile are kept for restoring.
    pub restore_pending: bool,
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
    /// Result of [`Request::ApplyProfile`] (also of a dry run).
    ProfileApplied {
        /// Human-readable progress, in order.
        lines: Vec<String>,
        /// Profile TOML that undoes the writes (with a comment header); `None` when
        /// nothing is (or would be) written. Sent in a dry run too. The agent never
        /// writes files: the client saves it.
        restore_toml: Option<String>,
        /// A write failed part-way; earlier writes were sent and `restore_toml`
        /// undoes them.
        error: Option<String>,
    },
    ShuttingDown,
    /// Answer to [`Request::ReloadAppProfiles`] and [`Request::AppProfiles`].
    AppProfiles(AppProfilesInfo),
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
    /// The device did not answer at all (`opensynapse_hid::HidError::NoResponse`):
    /// typically a sleeping wireless mouse. Clients may show a wake hint.
    NoResponse,
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
    /// The foreground application changed and the agent switched profiles.
    AppProfile {
        /// Foreground executable (full path).
        exe: String,
        /// Profile file applied; `None` = the values from before were restored.
        profile: Option<String>,
        /// Human-readable progress, in order.
        lines: Vec<String>,
        /// Why the switch did not (fully) happen; `None` = done.
        error: Option<String>,
    },
}
