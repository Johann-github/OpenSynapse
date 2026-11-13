//! Per-app profiles in the core (`vault/Decisions/ADR-0007 Per-app profiles.md`).
//!
//! The foreground watch posts `CoreMsg::Foreground`; the core looks the executable
//! up in `app-profiles.toml` and applies the mapped profile (or `default`) through
//! the shared engine, with volatile DPI-stage storage and skip-unchanged. Nothing
//! is written when the target profile is the active one. When no rule matches and
//! there is no `default`, the values from before the first app profile are
//! written back and forgotten. Running Razer software blocks every switch.
//!
//! Hotplug: a device that arrives while an app profile is active gets it (its
//! values at arrival become its restore profile, unless it already has older
//! ones); one that does not answer yet is retried on its next hardware event or
//! hotplug change, never on a timer. A removed device keeps its restore profile:
//! a wireless mouse keeps its volatile settings while its dongle is unplugged
//! (observed 2026-11-11), so its values at re-arrival are still the app's. When it
//! comes back with no app profile active, it gets its old values back.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use opensynapse_core::app_profiles::resolve_profile_path;
use opensynapse_core::{AppProfiles, Profile};
use opensynapse_device::profile_apply::ApplyOptions;
use opensynapse_ipc::msg::{AppProfilesInfo, AppRuleInfo, Event, Reply};
use opensynapse_protocol::dpi::Storage;

use super::Core;
use crate::hardware::Hardware;

/// App switching never forces and keeps DPI stages and button assignments in
/// volatile storage (button profile byte 0): no flash write, a power cycle brings
/// the stored values back.
const SWITCH_OPTIONS: ApplyOptions = ApplyOptions {
    storage: Storage::Volatile,
    all: false,
    force: false,
};

/// Per-app profile state, owned by the core.
#[derive(Default)]
pub struct AppState {
    /// `app-profiles.toml`; `None` when no location is known (no config folder).
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
    /// Devices that arrived and did not take the active app profile yet, or (no
    /// app profile active) returned and did not take their values from before yet.
    pending: BTreeSet<String>,
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
            tracing::info!(
                "no config folder (APPDATA / XDG_CONFIG_HOME / HOME); per-app profiles off"
            );
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
        // The switch covers every connected device; nothing waits any longer.
        self.apps.pending.clear();
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
        let (profile, pid) = match load_app_profile(path) {
            Ok(p) => p,
            Err(e) => return (Vec::new(), Some(e)),
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
            let (l, e) = self.apply_app_profile_to(&dev, profile.clone());
            lines.extend(l);
            if e.is_some() {
                error = e;
            }
        }
        (lines, error)
    }

    /// Apply `profile` to one device and keep the values read before it, merged
    /// "first value wins" into the device's restore profile.
    fn apply_app_profile_to(
        &mut self,
        dev: &str,
        profile: Profile,
    ) -> (Vec<String>, Option<String>) {
        match self.run_apply(dev, profile, SWITCH_OPTIONS, false) {
            Ok(r) => {
                // Keep the oldest value of each setting for the final restore;
                // also after a failed write (it undoes the writes that were sent).
                if let Some(old) = &r.restore {
                    self.apps
                        .restore
                        .entry(dev.to_owned())
                        .or_default()
                        .fill_missing_from(old);
                }
                (r.lines, r.failed)
            }
            Err(e) => (Vec::new(), Some(e.message)),
        }
    }

    /// The device's late apply failed and waits for a retry.
    pub(super) fn app_apply_pending(&self, dev: &str) -> bool {
        self.apps.pending.contains(dev)
    }

    /// The device is gone: drop any pending retry, keep its values from before.
    /// A wireless mouse keeps the app values while its dongle is out, so reading
    /// them again at re-arrival would lose the real "before" (observed 2026-11-11).
    pub(super) fn forget_app_device(&mut self, dev: &str) {
        self.apps.pending.remove(dev);
        if self.apps.restore.contains_key(dev) {
            tracing::info!(path = %dev, "device removed; its values from before the app profile are kept for its return");
        }
    }

    /// Give the active app profile to devices that were not there when it was
    /// switched to: the ones in `arrived` and the ones waiting for a retry. The
    /// values read before applying become the device's restore profile. A device
    /// that does not answer (e.g. a wireless mouse asleep or not linked to its
    /// dongle) is reported once and retried on its next hardware event or the next
    /// hotplug change; never on a timer.
    pub(super) fn apply_app_profile_late(&mut self, arrived: &[String]) {
        let Some(active) = self.apps.active.clone() else {
            // `pending` now holds returning devices whose restore failed.
            let mut back = std::mem::take(&mut self.apps.pending);
            back.extend(arrived.iter().cloned());
            self.restore_returning(back, arrived);
            return;
        };
        let mut targets = std::mem::take(&mut self.apps.pending);
        targets.extend(arrived.iter().cloned());
        targets.retain(|d| self.devices.contains_key(d));
        if targets.is_empty() {
            return;
        }
        let (profile, pid) = match load_app_profile(Path::new(&active)) {
            Ok(p) => p,
            Err(error) => {
                // The file changed since the switch; the next switch reports it
                // again, so no retry here.
                if !arrived.is_empty() {
                    tracing::error!(profile = %active, %error, "app profile not applied to new device");
                    self.broadcast_app_profile(Some(active), Vec::new(), Some(error));
                }
                return;
            }
        };
        // Devices with another PID have nothing to do.
        targets.retain(|d| self.devices[d].pid == pid);
        let conflict = self.hw.conflict();
        for dev in targets {
            let (lines, error) = match &conflict {
                Some(name) => (
                    Vec::new(),
                    Some(format!(
                        "{name} is running and may talk to the device at the same time; \
                         per-app profile not applied to {dev} (nothing written)"
                    )),
                ),
                None => self.apply_app_profile_to(&dev, profile.clone()),
            };
            match error {
                None => {
                    tracing::info!(path = %dev, profile = %active, "app profile applied to late device");
                    self.broadcast_app_profile(Some(active.clone()), lines, None);
                }
                Some(e) => {
                    // Reported once, at arrival; retries stay quiet until one works.
                    if arrived.contains(&dev) {
                        tracing::warn!(path = %dev, profile = %active, error = %e, "app profile not applied to new device; retry on its next event");
                        self.broadcast_app_profile(Some(active.clone()), lines, Some(e));
                    } else {
                        tracing::debug!(path = %dev, error = %e, "app profile retry failed");
                    }
                    self.apps.pending.insert(dev);
                }
            }
        }
    }

    fn broadcast_app_profile(
        &mut self,
        profile: Option<String>,
        lines: Vec<String>,
        error: Option<String>,
    ) {
        let exe = self.apps.foreground.clone().unwrap_or_default();
        self.broadcast(Event::AppProfile {
            exe,
            profile,
            lines,
            error,
        });
    }

    /// Devices that come back after the app profile ended while they were away:
    /// write their values from before. A failure keeps them and retries on the
    /// device's next event (reported once, at arrival), like a late apply.
    fn restore_returning(&mut self, mut back: BTreeSet<String>, arrived: &[String]) {
        back.retain(|d| self.devices.contains_key(d) && self.apps.restore.contains_key(d));
        if back.is_empty() {
            return;
        }
        if let Some(name) = self.hw.conflict() {
            let error = format!(
                "{name} is running and may talk to the device at the same time;                  values from before the app profile not restored (nothing written)"
            );
            tracing::warn!("{error}");
            if back.iter().any(|d| arrived.contains(d)) {
                self.broadcast_app_profile(None, Vec::new(), Some(error));
            }
            self.apps.pending.extend(back);
            return;
        }
        for dev in back {
            let profile = self.apps.restore.remove(&dev).expect("filtered above");
            let (lines, error) = match self.run_apply(&dev, profile.clone(), SWITCH_OPTIONS, false)
            {
                Ok(r) => (r.lines, r.failed),
                Err(e) => (Vec::new(), Some(e.message)),
            };
            let first = arrived.contains(&dev);
            match &error {
                None => {
                    tracing::info!(path = %dev, "returning device got its values from before the app profile")
                }
                Some(e) => {
                    tracing::warn!(path = %dev, error = %e, "restore of returning device failed; retry on its next event");
                    self.apps.restore.insert(dev.clone(), profile);
                    self.apps.pending.insert(dev);
                }
            }
            if error.is_none() || first {
                self.broadcast_app_profile(None, lines, error);
            }
        }
    }

    /// Write back the values from before the first app profile, then forget them.
    /// A device whose restore fails keeps its values for the next try; one that is
    /// not connected keeps them until it returns.
    fn restore_app_values(&mut self) -> (Vec<String>, Option<String>) {
        if self.apps.restore.is_empty() {
            return (vec!["Nothing to restore.".into()], None);
        }
        let mut lines = Vec::new();
        let mut error = None;
        for (dev, profile) in std::mem::take(&mut self.apps.restore) {
            if !self.devices.contains_key(&dev) {
                lines.push(format!(
                    "{dev} is not connected; its values from before are restored when it returns."
                ));
                self.apps.restore.insert(dev, profile);
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

/// Read an app-profile file; it must name its device PID.
fn load_app_profile(path: &Path) -> Result<(Profile, u16), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let profile = Profile::from_toml(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let pid = profile
        .device
        .as_ref()
        .map(|d| d.pid.0)
        .ok_or_else(|| format!("{}: per-app profiles need [device] pid", path.display()))?;
    Ok((profile, pid))
}

#[cfg(test)]
mod tests;
