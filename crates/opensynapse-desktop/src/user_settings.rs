//! "Settings" section: user preferences from `settings.toml`, shared with the CLI
//! (`opensynapse settings get|set`). Read once at start and written on each
//! change; never watched. Decision:
//! `vault/Decisions/ADR-0008 User settings and wake hint.md`.

use std::path::PathBuf;

use eframe::egui::{Color32, RichText, Ui};
use opensynapse_core::Settings;
use opensynapse_core::settings::Key;
use opensynapse_sys::config;

/// The settings file and what the GUI knows about it.
pub struct UserSettings {
    path: Option<PathBuf>,
    settings: Settings,
    /// The file exists but could not be read or parsed: shown, never overwritten.
    load_error: Option<String>,
    /// Result of the last save.
    save_error: Option<String>,
}

impl UserSettings {
    /// Read `settings.toml`; a missing file means defaults (wake hint off).
    pub fn load() -> Self {
        let path = config::settings_path();
        let (settings, load_error) = match path.as_deref().map(config::read_optional) {
            None => (Settings::default(), None),
            Some(Ok(text)) => parse(text.as_deref()),
            Some(Err(e)) => (Settings::default(), Some(e.to_string())),
        };
        Self {
            path,
            settings,
            load_error,
            save_error: None,
        }
    }

    /// The checkbox row. Disabled when the file cannot be used.
    pub fn ui(&mut self, ui: &mut Ui, bad: Color32) {
        let blocked = match (&self.path, &self.load_error) {
            (None, _) => Some("No per-user config folder (APPDATA / HOME not set)".to_owned()),
            (_, Some(e)) => Some(format!("{e}; fix or delete the file")),
            _ => None,
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Settings").strong());
            let mut on = self.settings.wake_hint;
            let r = ui.add_enabled(
                blocked.is_none(),
                eframe::egui::Checkbox::new(&mut on, "Show hint when the mouse is asleep"),
            );
            let r = match (&blocked, &self.path) {
                (Some(why), _) => r.on_disabled_hover_text(why.as_str()),
                (None, Some(p)) => r.on_hover_text(format!("Saved in {}", p.display())),
                (None, None) => r,
            };
            if r.changed() {
                self.set(Key::WakeHint, on);
            }
        });
        for e in [&self.load_error, &self.save_error].into_iter().flatten() {
            ui.colored_label(bad, e);
        }
    }

    fn set(&mut self, key: Key, on: bool) {
        let Some(path) = &self.path else { return };
        let mut next = self.settings.clone();
        next.set(key, on);
        let saved = next
            .to_toml()
            .map_err(|e| e.to_string())
            .and_then(|t| config::write(path, &t).map_err(|e| e.to_string()));
        match saved {
            Ok(()) => {
                self.settings = next;
                self.save_error = None;
            }
            Err(e) => self.save_error = Some(format!("Settings not saved: {e}")),
        }
    }

    /// The wake hint for a device error message, if the user turned it on.
    pub fn hint(&self, message: &str) -> Option<&'static str> {
        hint(&self.settings, message)
    }

    /// An error line in `color`, followed by the wake hint when it applies.
    pub fn error_label(&self, ui: &mut Ui, color: Color32, message: &str) {
        ui.colored_label(color, message);
        if let Some(h) = self.hint(message) {
            ui.label(RichText::new(h).strong());
        }
    }
}

/// File text (or `None` for a missing file) → settings and a load error.
fn parse(text: Option<&str>) -> (Settings, Option<String>) {
    match Settings::from_toml(text.unwrap_or("")) {
        Ok(s) => (s, None),
        Err(e) => (Settings::default(), Some(e.to_string())),
    }
}

/// Messages reach the GUI as text (direct HID and over the agent); the class
/// text comes from `opensynapse-hid`, so both backends are recognised.
fn hint(settings: &Settings, message: &str) -> Option<&'static str> {
    settings.wake_hint_for(opensynapse_hid::is_no_response_text(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensynapse_core::WAKE_HINT;
    use opensynapse_hid::HidError;

    fn asleep() -> String {
        format!(
            "device does not answer: {}",
            HidError::NoResponse { attempts: 20 }
        )
    }

    #[test]
    fn missing_file_means_hint_off() {
        let (s, err) = parse(None);
        assert!(err.is_none());
        assert_eq!(hint(&s, &asleep()), None);
    }

    #[test]
    fn hint_when_on_and_asleep_only() {
        let (s, err) = parse(Some("wake_hint = true\n"));
        assert!(err.is_none());
        assert_eq!(hint(&s, &asleep()), Some(WAKE_HINT));
        assert_eq!(hint(&s, "device answered NotSupported"), None);
    }

    #[test]
    fn invalid_file_is_reported_and_hint_off() {
        let (s, err) = parse(Some("wake_hint = 1"));
        assert!(err.is_some());
        assert_eq!(hint(&s, &asleep()), None);
    }
}
