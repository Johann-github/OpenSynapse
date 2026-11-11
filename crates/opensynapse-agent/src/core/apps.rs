//! Per-app profiles in the core (`vault/Decisions/ADR-0007 Per-app profiles.md`).
//!
//! The foreground watch posts `CoreMsg::Foreground`; the core looks the executable
//! up in `app-profiles.toml` and applies the mapped profile (or `default`) through
//! the shared engine, with volatile DPI-stage storage and skip-unchanged. Nothing
//! is written when the target profile is the active one. When no rule matches and
//! there is no `default`, the values from before the first app profile are
//! written back and forgotten. Running Razer software blocks every switch.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use opensynapse_core::app_profiles::resolve_profile_path;
use opensynapse_core::{AppProfiles, Profile};
use opensynapse_device::profile_apply::ApplyOptions;
use opensynapse_ipc::msg::{AppProfilesInfo, AppRuleInfo, Event, Reply};
use opensynapse_protocol::dpi::Storage;

use super::Core;
use crate::hardware::Hardware;

/// App switching never forces and keeps DPI stages in volatile storage (no flash
/// write for DPI; a power cycle brings the stored stages back).
const SWITCH_OPTIONS: ApplyOptions = ApplyOptions {
    storage: Storage::Volatile,
    all: false,
    force: false,
};

/// Per-app profile state, owned by the core.
#[derive(Default)]
pub struct AppState {
    /// `app-profiles.toml`; `None` when no location is known (no `APPDATA`).
    file: Option<PathBuf>,
    /// Loaded rules; `None` = feature off (file missing or invalid).
    rules: Option<AppProfiles>,
    /// Why the file was rejected.
    error: Option<String>,
    /// Last foreground executable.
    foreground: Option<String>,
    /// Resolved profile path applied by app switching.
    active: Option<String>,
    /// Device path → values from before the first app profile (first read wins).
    restore: BTreeMap<String, Profile>,
}

impl<H: Hardware> Core<H> {
    /// Where the rules live; call [`Self::load_app_profiles`] afterwards.
    pub fn set_app_profiles_file(&mut self, file: Option<PathBuf>) {
        self.apps.file = file;
    }

    /// (Re)read the rules file. Missing = off (info); invalid = off (error); the
    /// agent keeps running either way.
    pub fn load_app_profiles(&mut self) {
        let a = &mut self.apps;
        a.rules = None;
        a.error = None;
        let Some(file) = &a.file else {
            tracing::info!("no APPDATA folder; per-app profiles off");
            return;
        };
        match std::fs::read_to_string(file) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                tracing::info!(file = %file.display(), "no app-profiles file; per-app profiles off");
            }
            Err(e) => {
                let msg = format!("cannot read {}: {e}", file.display());
                tracing::error!("{msg}; per-app profiles off");
                a.error = Some(msg);
            }
            Ok(text) => match AppProfiles::from_toml(&text) {
                Ok(r) => {
                    tracing::info!(
                        file = %file.display(),
                        rules = r.apps.len(),
                        default = r.default.as_deref().unwrap_or("-"),
                        "per-app profiles on"
                    );
                    a.rules = Some(r);
                }
                Err(e) => {
                    let msg = format!("{}: {e}", file.display());
                    tracing::error!("{msg}; per-app profiles off");
                    a.error = Some(msg);
                }
            },
        }
    }

    /// `ReloadAppProfiles`: re-read, then re-evaluate the foreground application.
    pub(super) fn reload_app_profiles(&mut self) -> Reply {
        self.load_app_profiles();
        self.switch_app_profile();
        Reply::AppProfiles(self.app_profiles_info())
    }

    pub(super) fn app_profiles_info(&self) -> AppProfilesInfo {
        let a = &self.apps;
        let rules = a.rules.as_ref();
        AppProfilesInfo {
            config: a.file.as_ref().map(|f| f.display().to_string()),
            enabled: rules.is_some(),
            error: a.error.clone(),
            default: rules.and_then(|r| r.default.clone()),
            rules: rules
                .map(|r| {
                    r.apps
                        .iter()
                        .map(|r| AppRuleInfo {
                            exe: r.exe.clone(),
                            profile: r.profile.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            foreground: a.foreground.clone(),
            active: a.active.clone(),
            restore_pending: !a.restore.is_empty(),
        }
    }

    /// The foreground watch saw a new executable.
    pub(super) fn on_foreground(&mut self, exe: String) {
        tracing::debug!(%exe, "foreground");
        self.apps.foreground = Some(exe);
        self.switch_app_profile();
    }

    /// The profile the rules pick for the foreground executable (resolved path).
    fn app_target(&self) -> Option<PathBuf> {
        let a = &self.apps;
        let (rules, file, exe) = (a.rules.as_ref()?, a.file.as_ref()?, a.foreground.as_ref()?);
        rules
            .profile_for(exe)
            .map(|p| resolve_profile_path(file, p))
    }

    /// Bring the devices to what the rules say for the foreground application.
    /// No-op when that is already active.
    fn switch_app_profile(&mut self) {
        let Some(exe) = self.apps.foreground.clone() else {
            return;
        };
        let target = self.app_target();
        let key = target.as_ref().map(|p| p.display().to_string());
        if key == self.apps.active {
            return;
        }
        // Checked at switch time, never on a timer; nothing is touched while it runs.
        if let Some(name) = self.hw.conflict() {
            let error = format!(
                "{name} is running and may talk to the device at the same time; \
                 per-app profile not switched (nothing written)"
            );
            tracing::warn!(%exe, "{error}");
            self.broadcast(Event::AppProfile {
                exe,
                profile: key,
                lines: Vec::new(),
                error: Some(error),
            });
            return;
        }
        let (lines, error) = match &target {
            Some(path) => self.apply_app_profile(path),
            None => self.restore_app_values(),
        };
        match &error {
            None => {
                tracing::info!(%exe, profile = key.as_deref().unwrap_or("(restored)"), "app profile switched");
                self.apps.active = key.clone();
            }
            Some(e) => {
                tracing::error!(%exe, profile = key.as_deref().unwrap_or("(restore)"), error = %e, "app profile switch failed")
            }
        }
        for l in &lines {
            tracing::debug!("{l}");
        }
        self.broadcast(Event::AppProfile {
            exe,
            profile: key,
            lines,
            error,
        });
    }

    /// Apply the profile file at `path` to every connected device with its PID.
    fn apply_app_profile(&mut self, path: &Path) -> (Vec<String>, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                return (
                    Vec::new(),
                    Some(format!("cannot read {}: {e}", path.display())),
                );
            }
        };
        let profile = match Profile::from_toml(&text) {
            Ok(p) => p,
            Err(e) => return (Vec::new(), Some(format!("{}: {e}", path.display()))),
        };
        let Some(pid) = profile.device.as_ref().map(|d| d.pid.0) else {
            return (
                Vec::new(),
                Some(format!(
                    "{}: per-app profiles need [device] pid",
                    path.display()
                )),
            );
        };
        let targets: Vec<String> = self
            .devices
            .values()
            .filter(|e| e.pid == pid)
            .map(|e| e.path.clone())
            .collect();
        if targets.is_empty() {
            return (
                vec![format!(
                    "No connected device with PID {pid:04X}; nothing written."
                )],
                None,
            );
        }
        let mut lines = Vec::new();
        let mut error = None;
        for dev in targets {
            match self.run_apply(&dev, profile.clone(), SWITCH_OPTIONS, false) {
                Ok(r) => {
                    // Keep the oldest value of each setting for the final restore;
                    // also after a failed write (it undoes the writes that were sent).
                    if let Some(old) = &r.restore {
                        self.apps
                            .restore
                            .entry(dev.clone())
                            .or_default()
                            .fill_missing_from(old);
                    }
                    lines.extend(r.lines);
                    if let Some(f) = r.failed {
                        error = Some(f);
                    }
                }
                Err(e) => error = Some(e.message),
            }
        }
        (lines, error)
    }

    /// Write back the values from before the first app profile, then forget them.
    /// A device whose restore fails keeps its values for the next try.
    fn restore_app_values(&mut self) -> (Vec<String>, Option<String>) {
        if self.apps.restore.is_empty() {
            return (vec!["Nothing to restore.".into()], None);
        }
        let mut lines = Vec::new();
        let mut error = None;
        for (dev, profile) in std::mem::take(&mut self.apps.restore) {
            if !self.devices.contains_key(&dev) {
                lines.push(format!(
                    "{dev} is not connected; its values from before are dropped."
                ));
                continue;
            }
            match self.run_apply(&dev, profile.clone(), SWITCH_OPTIONS, false) {
                Ok(r) => {
                    lines.extend(r.lines);
                    if let Some(f) = r.failed {
                        error = Some(f);
                        self.apps.restore.insert(dev, profile);
                    }
                }
                Err(e) => {
                    error = Some(e.message);
                    self.apps.restore.insert(dev, profile);
                }
            }
        }
        (lines, error)
    }
}

#[cfg(test)]
mod tests;
