//! Pure types and TOML config for OpenSynapse. No I/O. See README.md.
//!
//! [`Profile`] is a device profile (TOML text in, TOML text out). [`Profile::validate`]
//! checks values against the protocol and [`DeviceLimits`]; [`Profile::plan`] turns a
//! valid profile into protocol requests built by `opensynapse-protocol`.
//! [`AppProfiles`] maps foreground applications to profile files (agent).
//! [`Settings`] holds user preferences of the CLI and GUI (`settings.toml`).

pub mod app_profiles;
pub mod plan;
pub mod profile;
pub mod settings;
pub mod validate;

pub use app_profiles::{AppProfiles, AppProfilesError, AppRule};
pub use plan::{PlannedWrite, PollingVariant, Setting, Target};
pub use profile::{
    ButtonAction, ButtonId, Buttons, DeviceSection, DpiSection, HyperShiftSection, Pid,
    PollingSection, PowerSection, Profile, TomlError,
};
pub use settings::{Settings, SettingsError, WAKE_HINT};
pub use validate::{DeviceLimits, Issue, ValidationError};

#[cfg(test)]
mod tests;
