//! `opensynapse settings get|set` and the optional wake hint on errors.
//!
//! The file is `%APPDATA%\OpenSynapse\settings.toml` (`opensynapse_sys::config`);
//! the format is `opensynapse_core::Settings`. Decision:
//! `vault/Decisions/ADR-0008 User settings and wake hint.md`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::{Subcommand, ValueEnum};
use opensynapse_core::Settings;
use opensynapse_core::settings::Key;
use opensynapse_hid::HidError;
use opensynapse_ipc::msg::ErrorKind;
use opensynapse_sys::config;

use crate::agent::AgentError;

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Show every setting and the file it comes from.
    Get,
    /// Change one setting and save the file.
    Set { key: SettingKey, value: OnOff },
}

/// Settings that can be changed by name.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKey {
    /// Print "move the mouse to wake it" when the device did not respond.
    WakeHint,
}

impl From<SettingKey> for Key {
    fn from(k: SettingKey) -> Self {
        match k {
            SettingKey::WakeHint => Key::WakeHint,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnOff {
    On,
    Off,
}

pub fn run(action: SettingsAction) -> Result<()> {
    let path = path()?;
    match action {
        SettingsAction::Get => {
            let (s, exists) = load(&path)?;
            print!("{}", render(&s, &path, exists));
        }
        SettingsAction::Set { key, value } => {
            // An unreadable or invalid file is reported, never overwritten.
            let (mut s, _) = load(&path)?;
            let key = Key::from(key);
            s.set(key, value == OnOff::On);
            config::write(&path, &s.to_toml()?)
                .with_context(|| format!("cannot save {}", path.display()))?;
            println!(
                "{} = {} ({})",
                key.cli_name(),
                on_off(s.get(key)),
                path.display()
            );
        }
    }
    Ok(())
}

fn path() -> Result<PathBuf> {
    config::settings_path().ok_or_else(|| anyhow!("no per-user config folder (APPDATA is not set)"))
}

/// Settings from `path`; a missing file gives the defaults (`exists` = false).
fn load(path: &Path) -> Result<(Settings, bool)> {
    match config::read_optional(path)? {
        None => Ok((Settings::default(), false)),
        Some(text) => Ok((
            Settings::from_toml(&text).with_context(|| path.display().to_string())?,
            true,
        )),
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn render(s: &Settings, path: &Path, exists: bool) -> String {
    let mut o = String::new();
    for k in Key::ALL {
        o.push_str(&format!("{} = {}\n", k.cli_name(), on_off(s.get(k))));
    }
    let note = if exists {
        ""
    } else {
        " (not created yet; defaults)"
    };
    o.push_str(&format!("File: {}{note}\n", path.display()));
    o
}

/// True when `e` is the "device did not respond" class: a typed
/// [`HidError::NoResponse`] from a direct HID command, an agent error of kind
/// [`ErrorKind::NoResponse`], or a message carrying the HID class text (write
/// failures and profile errors that crossed a string boundary).
pub fn is_no_response(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<HidError>()
            .is_some_and(HidError::is_no_response)
            || c.downcast_ref::<AgentError>()
                .is_some_and(|a| a.kind == ErrorKind::NoResponse)
    }) || opensynapse_hid::is_no_response_text(&format!("{e:#}"))
}

/// The wake hint for `e`, only when the user turned it on. Never fails: an
/// unreadable settings file means "off".
pub fn hint_for(e: &anyhow::Error) -> Option<&'static str> {
    if !is_no_response(e) {
        return None;
    }
    let settings = config::settings_path()
        .and_then(|p| load(&p).ok())
        .map(|(s, _)| s)
        .unwrap_or_default();
    settings.wake_hint_for(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use opensynapse_ipc::msg::IpcError;

    #[test]
    fn typed_hid_error_is_recognised_through_context() {
        let e = anyhow::Error::new(HidError::NoResponse { attempts: 20 });
        let e = Err::<(), _>(e).context("cannot read DPI").unwrap_err();
        assert!(is_no_response(&e));
    }

    #[test]
    fn agent_error_kind_is_recognised() {
        let e = anyhow::Error::new(AgentError::from(IpcError {
            kind: ErrorKind::NoResponse,
            message: "device does not answer".into(),
        }));
        assert!(is_no_response(&e));
        let other = anyhow::Error::new(AgentError::from(IpcError {
            kind: ErrorKind::Device,
            message: "short read".into(),
        }));
        assert!(!is_no_response(&other));
    }

    #[test]
    fn text_class_is_recognised() {
        let msg = format!("write failed: {}", HidError::NoResponse { attempts: 20 });
        assert!(is_no_response(&anyhow!(msg)));
        assert!(!is_no_response(&anyhow!("device answered NotSupported")));
    }

    #[test]
    fn render_lists_keys_and_file() {
        let mut s = Settings::default();
        let p = Path::new("settings.toml");
        assert_eq!(
            render(&s, p, false),
            "wake-hint = off\nFile: settings.toml (not created yet; defaults)\n"
        );
        s.wake_hint = true;
        assert_eq!(render(&s, p, true), "wake-hint = on\nFile: settings.toml\n");
    }
}
