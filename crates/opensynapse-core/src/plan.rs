//! Turn a [`Profile`] into the protocol requests that apply it. Pure: every request
//! comes from an `opensynapse_protocol` builder; no bytes are built here.

use std::fmt;

use opensynapse_protocol::{
    Request,
    buttons::{self, Mode},
    dpi::{self, Dpi, DpiStage, DpiStages, Storage},
    hid_usage, polling, power,
};

use crate::profile::{ButtonAction, ButtonId, Profile};
use crate::validate::{DeviceLimits, ValidationError};

/// Which polling command family the device speaks (unknown up front per device;
/// see `vault/Protocol/Commands 2.5.md`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PollingVariant {
    /// Period code (00/05).
    #[default]
    Period,
    /// HyperPolling bitmask (00/40).
    Hyper,
}

/// Device facts needed to plan writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub limits: DeviceLimits,
    /// Onboard profile slot that button (and HyperPolling) writes go to.
    pub onboard_profile: u8,
    /// DPI storage byte; `Persistent` survives power-off (observed).
    pub dpi_storage: Storage,
    pub polling: PollingVariant,
}

impl Default for Target {
    fn default() -> Self {
        Self {
            limits: DeviceLimits::default(),
            onboard_profile: 1,
            dpi_storage: Storage::Persistent,
            polling: PollingVariant::Period,
        }
    }
}

/// What a planned write changes, for logs and UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    DpiStages,
    PollingRate,
    SleepTime,
    Button { mode: Mode, button: u8 },
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Setting::DpiStages => f.write_str("DPI stages"),
            Setting::PollingRate => f.write_str("polling rate"),
            Setting::SleepTime => f.write_str("sleep time"),
            Setting::Button { mode, button } => write!(f, "button {button} ({mode} mode)"),
        }
    }
}

/// One write the profile asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedWrite {
    pub setting: Setting,
    pub request: Request,
}

impl Profile {
    /// Validate, then list the writes that apply this profile, in section order
    /// (DPI, polling, power, normal buttons, HyperShift buttons).
    pub fn plan(&self, target: &Target) -> Result<Vec<PlannedWrite>, ValidationError> {
        self.validate(&target.limits)?;
        let mut out = Vec::new();
        let mut push = |setting, request| out.push(PlannedWrite { setting, request });

        if let Some(d) = &self.dpi {
            let stages = DpiStages {
                // Stage IDs are 1-based positions (observed in stage reads), so the
                // active stage's ID equals its 1-based position.
                active: d.active_stage,
                stages: d
                    .stages
                    .iter()
                    .zip(1u8..)
                    .map(|(&v, id)| DpiStage {
                        id,
                        dpi: Dpi::uniform(v),
                    })
                    .collect(),
            };
            push(
                Setting::DpiStages,
                dpi::set_dpi_stages(target.dpi_storage, &stages).map_err(internal)?,
            );
        }

        if let Some(p) = &self.polling {
            let req = match target.polling {
                PollingVariant::Period => polling::set_polling_rate(p.hz),
                PollingVariant::Hyper => polling::set_hyper_polling(target.onboard_profile, p.hz),
            };
            push(Setting::PollingRate, req.map_err(internal)?);
        }

        if let Some(p) = &self.power {
            push(Setting::SleepTime, power::set_sleep_time(p.sleep_seconds));
        }

        for (mode, id, action) in self.all_buttons() {
            let a = assignment(target.onboard_profile, mode, id, action);
            push(
                Setting::Button { mode, button: id.0 },
                buttons::set_button_assignment(&a).map_err(internal)?,
            );
        }
        Ok(out)
    }
}

/// The protocol assignment for a validated action.
fn assignment(
    profile: u8,
    mode: Mode,
    id: ButtonId,
    action: &ButtonAction,
) -> buttons::ButtonAssignment {
    let b = id.0;
    match action {
        ButtonAction::MouseButton { target } => buttons::mouse_button(profile, b, mode, *target),
        ButtonAction::DoubleClick { target } => buttons::double_click(profile, b, mode, *target),
        ButtonAction::Key { key, mods } => {
            let usage = hid_usage::usage(key).expect("validated key name");
            let mask = mods
                .iter()
                .map(|m| hid_usage::modifier(m).expect("validated modifier"))
                .fold(0, |acc, bit| acc | bit);
            buttons::keyboard(profile, b, mode, mask, usage)
        }
        ButtonAction::DpiUp {} => buttons::dpi_up(profile, b, mode),
    }
}

/// A builder rejected a value that validation accepted: a bug, but report it as an
/// issue rather than panic.
fn internal(e: opensynapse_protocol::ProtocolError) -> ValidationError {
    ValidationError(vec![crate::validate::Issue {
        path: String::new(),
        message: format!("protocol rejected a validated value: {e}"),
    }])
}
