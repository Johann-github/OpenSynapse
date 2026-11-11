//! User settings (`settings.toml`): app behaviour, not device configuration.
//! Pure: text in, settings out. Where the file lives and reading/writing it is
//! up to the caller (`opensynapse_sys::settings_path`).
//! Decision: `vault/Decisions/ADR-0008 User settings and wake hint.md`.
//!
//! ```toml
//! wake_hint = false   # show "move the mouse to wake it" on a no-response error
//! ```
//!
//! A missing file and an empty file mean defaults. A missing key means its
//! default. Unknown keys are tolerated and written back unchanged, so a file
//! from a newer version survives a `set` from an older one.

use serde::{Deserialize, Serialize};

/// Shown next to a device error when [`Settings::wake_hint`] is on and the device
/// did not respond (`opensynapse_hid::HidError::NoResponse`).
pub const WAKE_HINT: &str = "Mouse seems asleep \u{2014} move it to wake it, then retry";

/// The parsed settings file. Every field has a default; add new ones the same way.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Show [`WAKE_HINT`] when a command fails because the device did not
    /// respond. Off unless the user turns it on.
    pub wake_hint: bool,
    /// Keys this version does not know, kept for writing back.
    #[serde(flatten)]
    pub unknown: toml::Table,
}

/// A setting the user can change by name (`opensynapse settings set <key>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    WakeHint,
}

impl Key {
    pub const ALL: [Key; 1] = [Key::WakeHint];

    /// Name on the command line (`wake-hint`).
    pub fn cli_name(self) -> &'static str {
        match self {
            Key::WakeHint => "wake-hint",
        }
    }

    /// Name in the file (`wake_hint`).
    pub fn file_name(self) -> &'static str {
        match self {
            Key::WakeHint => "wake_hint",
        }
    }
}

/// Why a settings file was rejected.
#[derive(Debug, thiserror::Error)]
#[error("invalid settings file: {0}")]
pub struct SettingsError(String);

impl Settings {
    /// Parse the file text; empty text gives the defaults.
    pub fn from_toml(text: &str) -> Result<Self, SettingsError> {
        toml::from_str(text).map_err(|e| SettingsError(e.to_string().trim_end().to_owned()))
    }

    pub fn to_toml(&self) -> Result<String, SettingsError> {
        toml::to_string(self).map_err(|e| SettingsError(e.to_string()))
    }

    /// Value of a boolean setting.
    pub fn get(&self, key: Key) -> bool {
        match key {
            Key::WakeHint => self.wake_hint,
        }
    }

    pub fn set(&mut self, key: Key, on: bool) {
        match key {
            Key::WakeHint => self.wake_hint = on,
        }
    }

    /// The hint to show for an error, if the user wants it and the error is
    /// the "device did not respond" class (`asleep`).
    pub fn wake_hint_for(&self, asleep: bool) -> Option<&'static str> {
        (self.wake_hint && asleep).then_some(WAKE_HINT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_empty_file_is_defaults() {
        let s = Settings::from_toml("").unwrap();
        assert_eq!(s, Settings::default());
        assert!(!s.wake_hint, "wake hint is off by default");
    }

    #[test]
    fn reads_wake_hint() {
        let s = Settings::from_toml("wake_hint = true\n").unwrap();
        assert!(s.wake_hint);
        assert!(s.get(Key::WakeHint));
    }

    #[test]
    fn unknown_keys_are_tolerated_and_kept() {
        let text = "future = 3\nwake_hint = true\n\n[later]\nx = \"y\"\n";
        let mut s = Settings::from_toml(text).unwrap();
        assert!(s.wake_hint);
        assert_eq!(s.unknown.len(), 2);
        s.set(Key::WakeHint, false);
        let back = Settings::from_toml(&s.to_toml().unwrap()).unwrap();
        assert!(!back.wake_hint);
        assert_eq!(back.unknown, s.unknown);
    }

    #[test]
    fn wrong_type_is_an_error() {
        let e = Settings::from_toml("wake_hint = \"yes\"").unwrap_err();
        assert!(e.to_string().starts_with("invalid settings file"), "{e}");
        assert!(Settings::from_toml("wake_hint = ").is_err());
    }

    #[test]
    fn round_trip() {
        let mut s = Settings::default();
        s.set(Key::WakeHint, true);
        let text = s.to_toml().unwrap();
        assert_eq!(text, "wake_hint = true\n");
        assert_eq!(Settings::from_toml(&text).unwrap(), s);
    }

    #[test]
    fn hint_only_when_on_and_asleep() {
        let mut s = Settings::default();
        assert_eq!(s.wake_hint_for(true), None);
        s.wake_hint = true;
        assert_eq!(s.wake_hint_for(false), None);
        assert_eq!(s.wake_hint_for(true), Some(WAKE_HINT));
    }

    #[test]
    fn key_names() {
        for k in Key::ALL {
            assert_eq!(k.cli_name().replace('-', "_"), k.file_name());
        }
    }
}
