//! `opensynapse-device` / `opensynapse-hid` types ↔ IPC wire types. Exhaustive
//! matches, so a new variant on either side fails to compile here.

use opensynapse_device::{self as dev, Setting, Snapshot, WriteFailure, WriteOpts, WriteOutcome};
use opensynapse_device_registry::{Device, Protocol, RAZER_VID, Registry};
use opensynapse_hid::HidEntry;
use opensynapse_ipc::msg::{
    DeviceInfo, Event, PollingVariant, WireSetting, WireSnapshot, WireWriteOpts, WriteDone,
    WriteFailed,
};
use opensynapse_protocol::event::{self as hw, Notification};

pub fn polling_to_wire(v: dev::PollingVariant) -> PollingVariant {
    match v {
        dev::PollingVariant::Period => PollingVariant::Period,
        dev::PollingVariant::Hyper => PollingVariant::Hyper,
    }
}

pub fn polling_from_wire(v: PollingVariant) -> dev::PollingVariant {
    match v {
        PollingVariant::Period => dev::PollingVariant::Period,
        PollingVariant::Hyper => dev::PollingVariant::Hyper,
    }
}

pub fn setting_to_wire(s: Setting) -> WireSetting {
    match s {
        Setting::Dpi(d) => WireSetting::Dpi(d),
        Setting::Polling { hz, variant } => WireSetting::Polling {
            hz,
            variant: polling_to_wire(variant),
        },
        Setting::Sleep(v) => WireSetting::Sleep(v),
        Setting::Button(a) => WireSetting::Button(a),
        Setting::DpiStages { storage, stages } => WireSetting::DpiStages { storage, stages },
    }
}

pub fn setting_from_wire(s: WireSetting) -> Setting {
    match s {
        WireSetting::Dpi(d) => Setting::Dpi(d),
        WireSetting::Polling { hz, variant } => Setting::Polling {
            hz,
            variant: polling_from_wire(variant),
        },
        WireSetting::Sleep(v) => Setting::Sleep(v),
        WireSetting::Button(a) => Setting::Button(a),
        WireSetting::DpiStages { storage, stages } => Setting::DpiStages { storage, stages },
    }
}

pub fn opts_from_wire(o: WireWriteOpts) -> WriteOpts {
    let WireWriteOpts {
        confirmed,
        allow_primary,
        restore,
    } = o;
    WriteOpts {
        confirmed,
        allow_primary,
        restore,
    }
}

pub fn snapshot_to_wire(s: Snapshot) -> WireSnapshot {
    let Snapshot {
        firmware,
        serial,
        mode,
        battery,
        charging,
        dpi,
        stages,
        polling,
        sleep,
        buttons_normal,
        buttons_hypershift,
    } = s;
    WireSnapshot {
        firmware: Ok(firmware),
        serial,
        mode,
        battery,
        charging,
        dpi,
        stages,
        polling: polling.map(|(hz, v)| (hz, polling_to_wire(v))),
        sleep,
        buttons_normal,
        buttons_hypershift,
    }
}

pub fn outcome_to_wire(o: WriteOutcome) -> WriteDone {
    let verified = o.verified();
    let WriteOutcome { old, new, now } = o;
    WriteDone {
        old: setting_to_wire(old),
        new: setting_to_wire(new),
        now: setting_to_wire(now),
        verified,
    }
}

pub fn failure_to_wire(f: WriteFailure) -> WriteFailed {
    let WriteFailure { old, message } = f;
    WriteFailed {
        old: old.map(setting_to_wire),
        message,
    }
}

/// Registry facts about a PID: (name, protocol).
pub fn registry_facts(pid: u16) -> (Option<&'static str>, Protocol) {
    match Registry::embedded().lookup(RAZER_VID, pid) {
        Some(m) => (Some(m.device().name.as_str()), m.protocol()),
        None => (None, Protocol::Unknown),
    }
}

/// The registry entry used to validate writes.
pub fn registry_device(pid: u16) -> Option<&'static Device> {
    Registry::embedded()
        .lookup(RAZER_VID, pid)
        .map(|m| m.device())
}

/// Only protocol 2.5 is implemented.
pub fn is_supported(protocol: Protocol) -> bool {
    protocol == Protocol::V25
}

pub fn device_info(e: &HidEntry) -> DeviceInfo {
    let (name, protocol) = registry_facts(e.pid);
    DeviceInfo {
        path: e.path.clone(),
        pid: e.pid,
        name: name.map(str::to_owned),
        protocol: protocol.to_string(),
        supported: is_supported(protocol),
    }
}

/// A decoded hardware notification as a subscriber event. `raw` is the input
/// report it came from (record ID, event ID, payload).
#[cfg_attr(not(test), allow(dead_code))] // used by EventSink (no source in slice 1)
pub fn notification_to_wire(path: &str, raw: &[u8], n: Notification) -> Event {
    let path = path.to_owned();
    let raw_event = |path: String| Event::Hardware {
        path,
        id: raw.get(1).copied().unwrap_or(0),
        payload: raw.get(2..).unwrap_or_default().to_vec(),
    };
    match n.event {
        hw::Event::Battery {
            level, charging, ..
        } => Event::Battery {
            path,
            level,
            charging,
        },
        hw::Event::DpiChanged { x, y } => Event::DpiChanged { path, x, y },
        hw::Event::Other { id, payload } => Event::Hardware { path, id, payload },
        hw::Event::ProfileChanged(_)
        | hw::Event::Lifted
        | hw::Event::PutDown
        | hw::Event::WirelessChanged { .. }
        | hw::Event::ExternalPower(_)
        | hw::Event::ScrollModeChanged(_)
        | hw::Event::BatteryFull
        | hw::Event::ConnectionQuality(_)
        | hw::Event::PollingRateChanged(_) => raw_event(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensynapse_protocol::dpi::{Dpi, Storage};

    #[test]
    fn settings_round_trip() {
        let all = [
            Setting::Dpi(Dpi::uniform(1600)),
            Setting::Polling {
                hz: 1000,
                variant: dev::PollingVariant::Hyper,
            },
            Setting::Sleep(300),
            Setting::Button(opensynapse_protocol::buttons::mouse_button(
                1,
                4,
                opensynapse_protocol::buttons::Mode::Normal,
                5,
            )),
            Setting::DpiStages {
                storage: Storage::Persistent,
                stages: dev::stages_from(&[400, 800], 0).unwrap(),
            },
        ];
        for s in all {
            assert_eq!(setting_from_wire(setting_to_wire(s.clone())), s);
        }
    }

    #[test]
    fn notifications() {
        let raw = [5, 0x31, 200, 1, 0, 1];
        let n = hw::decode(&raw).unwrap();
        assert_eq!(
            notification_to_wire("p", &raw, n),
            Event::Battery {
                path: "p".into(),
                level: 200,
                charging: 1
            }
        );
        let raw = [5, 0x05];
        let n = hw::decode(&raw).unwrap();
        assert_eq!(
            notification_to_wire("p", &raw, n),
            Event::Hardware {
                path: "p".into(),
                id: 5,
                payload: vec![]
            }
        );
    }

    #[test]
    fn viper_is_supported() {
        let info = device_info(&crate::hardware::fake::control_entry(
            "x",
            crate::hardware::fake::VIPER_PID,
        ));
        assert!(info.supported);
        assert_eq!(info.protocol, "2.5");
        assert!(info.name.is_some());
        assert!(!device_info(&crate::hardware::fake::control_entry("y", 0xFFFF)).supported);
    }
}
