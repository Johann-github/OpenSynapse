//! Device profile: the settings one device should have, stored as TOML.
//!
//! Every section is optional; a profile only sets what it lists. Parsing checks
//! shape and types; [`Profile::validate`](crate::validate) checks values.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A device profile. See the crate README for the TOML schema.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dpi: Option<DpiSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polling: Option<PollingSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerSection>,
    /// Normal-mode button assignments, keyed by button ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub buttons: Buttons,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hypershift: Option<HyperShiftSection>,
}

pub type Buttons = BTreeMap<ButtonId, ButtonAction>;

/// Which device the profile is meant for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSection {
    pub pid: Pid,
}

/// USB product ID, written as four hex digits (`"00B8"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Pid(pub u16);

impl TryFrom<String> for Pid {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let hex = s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .unwrap_or(&s);
        if hex.len() != 4 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("PID {s:?} must be four hex digits, e.g. \"00B8\""));
        }
        u16::from_str_radix(hex, 16)
            .map(Pid)
            .map_err(|e| e.to_string())
    }
}

impl From<Pid> for String {
    fn from(p: Pid) -> String {
        p.to_string()
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04X}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DpiSection {
    /// DPI per stage (same value for X and Y), in stage order.
    pub stages: Vec<u16>,
    /// 1-based position in `stages` of the stage that is active.
    pub active_stage: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollingSection {
    pub hz: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerSection {
    /// Idle time before the device sleeps. Unit is inferred to be seconds.
    pub sleep_seconds: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HyperShiftSection {
    /// HyperShift-mode button assignments, keyed by button ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub buttons: Buttons,
}

/// Device button ID as used by the protocol (`4` = first side button on the Viper).
/// A TOML table key, so it is written as a string (`[buttons.4]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ButtonId(pub u8);

impl TryFrom<String> for ButtonId {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        s.parse::<u8>()
            .map(ButtonId)
            .map_err(|_| format!("button ID {s:?} must be a number 0-255"))
    }
}

impl From<ButtonId> for String {
    fn from(b: ButtonId) -> String {
        b.0.to_string()
    }
}

impl fmt::Display for ButtonId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What a button does. Tagged by `action`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ButtonAction {
    /// Act as mouse button `target` (1 = left … 8).
    MouseButton { target: u8 },
    /// Double click of mouse button `target` (1 = left).
    DoubleClick { target: u8 },
    /// Press keyboard `key` (name from `hid_usage`) with `mods` held.
    Key {
        key: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mods: Vec<String>,
    },
    /// Step DPI up.
    // Empty struct (not unit) so `deny_unknown_fields` rejects stray keys.
    DpiUp {},
}

/// TOML parse or serialize failure.
#[derive(Debug, thiserror::Error)]
pub enum TomlError {
    #[error("invalid profile: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("cannot write profile: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl Profile {
    /// Parse a profile from TOML text. Checks shape only; call
    /// [`Profile::validate`] for value checks.
    pub fn from_toml(text: &str) -> Result<Self, TomlError> {
        Ok(toml::from_str(text)?)
    }

    pub fn to_toml(&self) -> Result<String, TomlError> {
        Ok(toml::to_string(self)?)
    }

    /// Take from `other` every setting `self` does not set yet; settings already
    /// in `self` stay. Used to accumulate the values from before a series of
    /// applies: the first value read for a setting is the one to restore.
    pub fn fill_missing_from(&mut self, other: &Profile) {
        fn take<T: Clone>(mine: &mut Option<T>, theirs: &Option<T>) {
            if mine.is_none() {
                mine.clone_from(theirs);
            }
        }
        take(&mut self.device, &other.device);
        take(&mut self.dpi, &other.dpi);
        take(&mut self.polling, &other.polling);
        take(&mut self.power, &other.power);
        for (id, a) in &other.buttons {
            self.buttons.entry(*id).or_insert_with(|| a.clone());
        }
        if let Some(h) = &other.hypershift {
            let mine = self
                .hypershift
                .get_or_insert_with(HyperShiftSection::default);
            for (id, a) in &h.buttons {
                mine.buttons.entry(*id).or_insert_with(|| a.clone());
            }
        }
    }

    /// Normal-mode and HyperShift-mode buttons as one iterator.
    pub fn all_buttons(
        &self,
    ) -> impl Iterator<Item = (opensynapse_protocol::buttons::Mode, ButtonId, &ButtonAction)> {
        use opensynapse_protocol::buttons::Mode;
        let hs = self.hypershift.iter().flat_map(|h| h.buttons.iter());
        self.buttons
            .iter()
            .map(|(id, a)| (Mode::Normal, *id, a))
            .chain(hs.map(|(id, a)| (Mode::HyperShift, *id, a)))
    }
}
