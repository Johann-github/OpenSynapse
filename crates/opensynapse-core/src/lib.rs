//! Pure types and TOML config for OpenSynapse. No I/O. See README.md.
//!
//! [`Profile`] is a device profile (TOML text in, TOML text out). [`Profile::validate`]
//! checks values against the protocol and [`DeviceLimits`]; [`Profile::plan`] turns a
//! valid profile into protocol requests built by `opensynapse-protocol`.

pub mod plan;
pub mod profile;
pub mod validate;

pub use plan::{PlannedWrite, PollingVariant, Setting, Target};
pub use profile::{
    ButtonAction, ButtonId, Buttons, DeviceSection, DpiSection, HyperShiftSection, Pid,
    PollingSection, PowerSection, Profile, TomlError,
};
pub use validate::{DeviceLimits, Issue, ValidationError};

#[cfg(test)]
mod tests;
