//! Device reads and safe writes, independent of the UI and of the real backend.
//!
//! Everything here is generic over [`FeatureIo`], so it runs against the real
//! device in the worker thread and against a fake mouse in the tests. Report bytes
//! come only from `opensynapse-protocol` builders.
//!
//! Write safety mirrors the CLI: check the value against the registry, refuse while
//! Razer software runs unless the user confirmed, read the old value, write, read
//! back, and hand the old value to the UI so it can offer "Restore".

use std::fmt;

use opensynapse_device_registry::Device;
use opensynapse_hid::{FeatureIo, HidError, Session};
use opensynapse_protocol::buttons::{self, ButtonReply, Mode};
use opensynapse_protocol::device::{self, DeviceMode, FirmwareVersion};
use opensynapse_protocol::power::{self, BatteryLevel, ChargingStatus};
use opensynapse_protocol::{ProtocolError, Request, Status, dpi, polling};

/// Profile byte for the HyperPolling variant (same default as the CLI; meaning
/// unconfirmed).
pub const HYPER_PROFILE: u8 = 0;
/// Onboard profile whose button assignments are shown (CLI default).
pub const BUTTON_PROFILE: u8 = 1;
/// Side buttons shown in the Buttons group.
pub const SHOWN_BUTTONS: [u8; 2] = [4, 5];
/// Polling rates offered when the registry lists none.
pub const FALLBACK_POLLING_RATES: [u32; 3] = [125, 500, 1000];

/// A value read from the device, or why it could not be read.
pub type Field<T> = Result<T, String>;

/// Which polling-rate command pair the device answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollingVariant {
    /// 00/05 + 00/85.
    Period,
    /// 00/40 + 00/C0.
    Hyper,
}

impl fmt::Display for PollingVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Period => "period",
            Self::Hyper => "hyper",
        })
    }
}

/// One writable setting with its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// Active DPI, volatile storage (the CLI default; lost on power-off).
    Dpi(dpi::Dpi),
    Polling {
        hz: u32,
        variant: PollingVariant,
    },
    /// Sleep timer in seconds.
    Sleep(u16),
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dpi(d) => write!(f, "DPI {d}"),
            Self::Polling { hz, .. } => write!(f, "polling rate {hz} Hz"),
            Self::Sleep(v) => write!(f, "sleep timer {v}"),
        }
    }
}

impl Setting {
    fn write_request(&self) -> Result<Request, ProtocolError> {
        match *self {
            Self::Dpi(d) => Ok(dpi::set_dpi(dpi::Storage::Volatile, d)),
            Self::Polling {
                hz,
                variant: PollingVariant::Period,
            } => polling::set_polling_rate(hz),
            Self::Polling {
                hz,
                variant: PollingVariant::Hyper,
            } => polling::set_hyper_polling(HYPER_PROFILE, hz),
            Self::Sleep(v) => Ok(power::set_sleep_time(v)),
        }
    }

    /// Read the current value of the same setting (same polling variant).
    fn read_current<D: FeatureIo>(&self, s: &mut Session<D>) -> Result<Setting, HidError> {
        Ok(match *self {
            Self::Dpi(_) => Self::Dpi(query(
                s,
                &dpi::get_dpi(dpi::Storage::Volatile),
                dpi::parse_dpi,
            )?),
            Self::Polling { variant, .. } => Self::Polling {
                hz: read_polling_variant(s, variant)?,
                variant,
            },
            Self::Sleep(_) => {
                Self::Sleep(query(s, &power::get_sleep_time(), power::parse_sleep_time)?)
            }
        })
    }
}

fn query<D: FeatureIo, T>(
    s: &mut Session<D>,
    req: &Request,
    parse: impl FnOnce(&[u8]) -> Result<T, ProtocolError>,
) -> Result<T, HidError> {
    let reply = s.execute(req)?;
    Ok(parse(reply.args())?)
}

/// Keep a per-field error as text for the panel.
fn f<T>(r: Result<T, HidError>) -> Field<T> {
    r.map_err(|e| e.to_string())
}

fn read_polling_variant<D: FeatureIo>(
    s: &mut Session<D>,
    variant: PollingVariant,
) -> Result<u32, HidError> {
    match variant {
        PollingVariant::Period => {
            query(s, &polling::get_polling_rate(), polling::parse_polling_rate)
        }
        PollingVariant::Hyper => query(
            s,
            &polling::get_hyper_polling(HYPER_PROFILE),
            polling::parse_hyper_polling,
        ),
    }
}

/// Polling rate via the period variant, falling back to HyperPolling when the
/// device rejects the period command (same rule as the CLI).
pub fn read_polling<D: FeatureIo>(s: &mut Session<D>) -> Result<(u32, PollingVariant), HidError> {
    match read_polling_variant(s, PollingVariant::Period) {
        Ok(hz) => Ok((hz, PollingVariant::Period)),
        Err(HidError::Status(
            Status::NotSupported | Status::UnknownCommand | Status::WrongDeviceMode,
        )) => Ok((
            read_polling_variant(s, PollingVariant::Hyper)?,
            PollingVariant::Hyper,
        )),
        Err(e) => Err(e),
    }
}

/// Everything the read-only panel shows.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub firmware: FirmwareVersion,
    pub serial: Field<String>,
    pub mode: Field<(DeviceMode, u8)>,
    pub battery: Field<BatteryLevel>,
    pub charging: Field<ChargingStatus>,
    pub dpi: Field<dpi::Dpi>,
    pub stages: Field<dpi::DpiStages>,
    pub polling: Field<(u32, PollingVariant)>,
    pub sleep: Field<u16>,
    pub buttons: Vec<(u8, Field<ButtonReply>)>,
}

/// Read the whole panel. The firmware read goes first: if the device does not
/// answer it, give up at once instead of retrying every other field.
pub fn read_snapshot<D: FeatureIo>(s: &mut Session<D>) -> Result<Snapshot, String> {
    let firmware = query(
        s,
        &device::get_firmware_version(),
        device::parse_firmware_version,
    )
    .map_err(|e| format!("device does not answer: {e}"))?;

    Ok(Snapshot {
        firmware,
        serial: f(query(s, &device::get_serial(), device::parse_serial)),
        mode: f(query(
            s,
            &device::get_device_mode(),
            device::parse_device_mode,
        )),
        battery: f(query(
            s,
            &power::get_battery_level(),
            power::parse_battery_level,
        )),
        charging: f(query(
            s,
            &power::get_charging_status(),
            power::parse_charging_status,
        )),
        dpi: f(query(
            s,
            &dpi::get_dpi(dpi::Storage::Volatile),
            dpi::parse_dpi,
        )),
        stages: f(query(
            s,
            &dpi::get_dpi_stages(dpi::Storage::Volatile),
            dpi::parse_dpi_stages,
        )),
        polling: f(read_polling(s)),
        sleep: f(query(s, &power::get_sleep_time(), power::parse_sleep_time)),
        buttons: SHOWN_BUTTONS
            .iter()
            .map(|&b| {
                let req = buttons::get_button_assignment(BUTTON_PROFILE, b, Mode::Normal);
                let parse = |a: &[u8]| buttons::parse_button_assignment(a, Mode::Normal);
                (b, f(query(s, &req, parse)))
            })
            .collect(),
    })
}

/// Reject values outside what the registry says the device supports. Unknown
/// devices get no writes at all (the GUI has no `--force`).
pub fn validate(new: &Setting, device: Option<&Device>) -> Result<(), String> {
    let Some(d) = device else {
        return Err("device is not in the registry; writes are disabled".into());
    };
    match *new {
        Setting::Dpi(v) => {
            for x in [v.x, v.y] {
                if !d.dpi_valid(u32::from(x)) {
                    return Err(format!(
                        "{x} DPI is outside {}-{} (step {}) for this device",
                        d.dpi_min.unwrap_or(0),
                        d.dpi_max.unwrap_or(0),
                        d.dpi_step.unwrap_or(1)
                    ));
                }
            }
        }
        Setting::Polling { hz, .. } => {
            let offered: &[u32] = if d.polling_rates.is_empty() {
                &FALLBACK_POLLING_RATES
            } else {
                &d.polling_rates
            };
            if !offered.contains(&hz) {
                return Err(format!(
                    "{hz} Hz is not offered for this device ({offered:?})"
                ));
            }
        }
        Setting::Sleep(0) => return Err("sleep timer 0 is untested; refusing".into()),
        Setting::Sleep(_) => {}
    }
    Ok(())
}

/// Writes are refused while Razer software runs unless the user confirmed.
pub fn conflict_gate(conflict: Option<&str>, confirmed: bool) -> Result<(), String> {
    match conflict {
        Some(name) if !confirmed => Err(format!(
            "{name} is running and may talk to the device at the same time; \
             close it or tick the confirmation box"
        )),
        _ => Ok(()),
    }
}

/// A finished write: the value before, the value asked for, the value read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteOutcome {
    pub old: Setting,
    pub new: Setting,
    pub now: Setting,
}

impl WriteOutcome {
    pub fn verified(&self) -> bool {
        self.now == self.new
    }
}

/// A write that did not finish. `old` is set when the old value was read, so the
/// UI can still offer to restore it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteFailure {
    pub old: Option<Setting>,
    pub message: String,
}

impl WriteFailure {
    fn before_read(message: String) -> Self {
        Self { old: None, message }
    }
}

/// Validate → conflict gate → read old → write → read back.
pub fn guarded_write<D: FeatureIo>(
    s: &mut Session<D>,
    new: Setting,
    device: Option<&Device>,
    conflict: Option<&str>,
    confirmed: bool,
) -> Result<WriteOutcome, WriteFailure> {
    validate(&new, device).map_err(WriteFailure::before_read)?;
    conflict_gate(conflict, confirmed).map_err(WriteFailure::before_read)?;
    let req = new
        .write_request()
        .map_err(|e| WriteFailure::before_read(e.to_string()))?;
    let old = new
        .read_current(s)
        .map_err(|e| WriteFailure::before_read(format!("cannot read the current value: {e}")))?;
    let fail = |what: &str, e: HidError| WriteFailure {
        old: Some(old),
        message: format!("{what}: {e}"),
    };
    s.execute(&req).map_err(|e| fail("write failed", e))?;
    let now = new
        .read_current(s)
        .map_err(|e| fail("read-back failed", e))?;
    Ok(WriteOutcome { old, new, now })
}

#[cfg(test)]
mod tests;
