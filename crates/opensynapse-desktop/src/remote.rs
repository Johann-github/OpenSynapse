//! IPC wire types (`opensynapse-ipc`) <-> the GUI model (`opensynapse-device`).
//!
//! Mirrors the agent's own conversions in the other direction, without depending
//! on the agent crate. Exhaustive matches, so a new variant on either side fails
//! to compile here.

use opensynapse_ipc::msg::{
    IpcError, PollingVariant as WirePolling, Reply, WireSetting, WireSnapshot, WireWriteOpts,
    WriteDone, WriteFailed,
};

use crate::ops::{PollingVariant, Setting, Snapshot, WriteFailure, WriteOpts, WriteOutcome};

pub fn polling_to_wire(v: PollingVariant) -> WirePolling {
    match v {
        PollingVariant::Period => WirePolling::Period,
        PollingVariant::Hyper => WirePolling::Hyper,
    }
}

pub fn polling_from_wire(v: WirePolling) -> PollingVariant {
    match v {
        WirePolling::Period => PollingVariant::Period,
        WirePolling::Hyper => PollingVariant::Hyper,
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

pub fn opts_to_wire(o: WriteOpts) -> WireWriteOpts {
    let WriteOpts {
        confirmed,
        allow_primary,
        restore,
    } = o;
    WireWriteOpts {
        confirmed,
        allow_primary,
        restore,
    }
}

/// The panel model. A firmware read that failed means the device did not answer
/// at all, the same as a failed local `read_snapshot`.
pub fn snapshot_from_wire(s: WireSnapshot) -> Result<Snapshot, String> {
    let WireSnapshot {
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
    let firmware = firmware.map_err(|e| format!("device does not answer: {e}"))?;
    Ok(Snapshot {
        firmware,
        serial,
        mode,
        battery,
        charging,
        dpi,
        stages,
        polling: polling.map(|(hz, v)| (hz, polling_from_wire(v))),
        sleep,
        buttons_normal,
        buttons_hypershift,
    })
}

/// `verified` is recomputed from `now == new`, as the local path does.
pub fn outcome_from_wire(d: WriteDone) -> WriteOutcome {
    let WriteDone { old, new, now, .. } = d;
    WriteOutcome {
        old: setting_from_wire(old),
        new: setting_from_wire(new),
        now: setting_from_wire(now),
    }
}

pub fn failure_from_wire(f: WriteFailed) -> WriteFailure {
    let WriteFailed { old, message } = f;
    WriteFailure {
        old: old.map(setting_from_wire),
        message,
    }
}

fn unexpected(reply: &Reply) -> String {
    format!("agent sent an unexpected reply: {reply:?}")
}

/// Answer to `ReadSnapshot`.
pub fn snapshot_reply(r: Result<Reply, IpcError>) -> Result<Snapshot, String> {
    match r {
        Ok(Reply::Snapshot(s)) => snapshot_from_wire(s),
        Ok(other) => Err(unexpected(&other)),
        Err(e) => Err(e.message),
    }
}

/// Answer to `Write`. A refused request (conflict, unsupported, no such device)
/// failed before anything was read.
pub fn written_reply(r: Result<Reply, IpcError>) -> Result<WriteOutcome, WriteFailure> {
    let before_read = |message| WriteFailure { old: None, message };
    match r {
        Ok(Reply::Written(w)) => w.map(outcome_from_wire).map_err(failure_from_wire),
        Ok(other) => Err(before_read(unexpected(&other))),
        Err(e) => Err(before_read(e.message)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use opensynapse_ipc::msg::ErrorKind;
    use opensynapse_protocol::buttons::{Mode, mouse_button};
    use opensynapse_protocol::device::{DeviceMode, FirmwareVersion};
    use opensynapse_protocol::dpi::{Dpi, Storage};
    use opensynapse_protocol::power::{BatteryLevel, ChargingStatus};

    fn all_settings() -> Vec<Setting> {
        vec![
            Setting::Dpi(Dpi::uniform(1600)),
            Setting::Polling {
                hz: 1000,
                variant: PollingVariant::Hyper,
            },
            Setting::Polling {
                hz: 500,
                variant: PollingVariant::Period,
            },
            Setting::Sleep(300),
            Setting::Button(mouse_button(1, 4, Mode::Normal, 5)),
            Setting::DpiStages {
                storage: Storage::Persistent,
                stages: crate::ops::stages_from(&[400, 800], 1).unwrap(),
            },
        ]
    }

    pub fn wire_snapshot() -> WireSnapshot {
        WireSnapshot {
            firmware: Ok(FirmwareVersion { major: 1, minor: 2 }),
            serial: Ok("PM123".into()),
            mode: Ok((DeviceMode::Normal, 0)),
            battery: Ok(BatteryLevel(128)),
            charging: Err("no answer".into()),
            dpi: Ok(Dpi::uniform(800)),
            stages: Ok(crate::ops::stages_from(&[400, 800, 1600], 1).unwrap()),
            polling: Ok((1000, WirePolling::Hyper)),
            sleep: Ok(300),
            buttons_normal: Ok(vec![mouse_button(1, 4, Mode::Normal, 4)]),
            buttons_hypershift: Err("not read".into()),
        }
    }

    #[test]
    fn settings_round_trip() {
        for s in all_settings() {
            assert_eq!(setting_from_wire(setting_to_wire(s.clone())), s);
        }
    }

    #[test]
    fn opts_keep_every_flag() {
        let o = WriteOpts {
            confirmed: true,
            allow_primary: false,
            restore: true,
        };
        assert_eq!(
            opts_to_wire(o),
            WireWriteOpts {
                confirmed: true,
                allow_primary: false,
                restore: true
            }
        );
    }

    #[test]
    fn snapshot_fields_carry_over() {
        let w = wire_snapshot();
        let s = snapshot_from_wire(w.clone()).unwrap();
        assert_eq!(s.firmware, FirmwareVersion { major: 1, minor: 2 });
        assert_eq!(s.serial, w.serial);
        assert_eq!(s.mode, w.mode);
        assert_eq!(s.battery, w.battery);
        assert_eq!(s.charging, Err::<ChargingStatus, _>("no answer".into()));
        assert_eq!(s.dpi, w.dpi);
        assert_eq!(s.stages, w.stages);
        assert_eq!(s.polling, Ok((1000, PollingVariant::Hyper)));
        assert_eq!(s.sleep, Ok(300));
        assert_eq!(s.buttons_normal, w.buttons_normal);
        assert_eq!(s.buttons_hypershift, w.buttons_hypershift);
    }

    #[test]
    fn snapshot_without_firmware_is_an_error() {
        let mut w = wire_snapshot();
        w.firmware = Err("timeout".into());
        let e = snapshot_from_wire(w).unwrap_err();
        assert!(e.contains("timeout"), "{e}");
    }

    #[test]
    fn write_done_and_failed() {
        let old = Setting::Sleep(300);
        let new = Setting::Sleep(600);
        let done = WriteDone {
            old: setting_to_wire(old.clone()),
            new: setting_to_wire(new.clone()),
            now: setting_to_wire(old.clone()),
            // The flag on the wire is ignored; `verified()` recomputes it.
            verified: true,
        };
        let o = outcome_from_wire(done);
        assert_eq!((&o.old, &o.new, &o.now), (&old, &new, &old));
        assert!(!o.verified());

        let f = failure_from_wire(WriteFailed {
            old: Some(setting_to_wire(old.clone())),
            message: "read-back failed".into(),
        });
        assert_eq!(f.old, Some(old));
        assert_eq!(f.message, "read-back failed");
    }

    #[test]
    fn replies() {
        assert!(snapshot_reply(Ok(Reply::Snapshot(wire_snapshot()))).is_ok());
        let err = IpcError {
            kind: ErrorKind::Conflict,
            message: "Synapse runs".into(),
        };
        assert_eq!(
            snapshot_reply(Err(err.clone())).unwrap_err(),
            "Synapse runs"
        );
        assert!(
            snapshot_reply(Ok(Reply::ShuttingDown))
                .unwrap_err()
                .contains("unexpected")
        );

        let f = written_reply(Err(err)).unwrap_err();
        assert_eq!((f.old, f.message.as_str()), (None, "Synapse runs"));
        let w = written_reply(Ok(Reply::Written(Err(WriteFailed {
            old: Some(WireSetting::Sleep(5)),
            message: "x".into(),
        }))))
        .unwrap_err();
        assert_eq!(w.old, Some(Setting::Sleep(5)));
        assert!(
            written_reply(Ok(Reply::Pong {
                agent_version: "0".into()
            }))
            .is_err()
        );
    }
}
