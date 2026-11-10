//! Value checks for a [`Profile`]. Device-specific ranges come in as [`DeviceLimits`]
//! so this crate stays independent of the device registry.

use std::fmt;

use opensynapse_protocol::{buttons::Layer, dpi::MAX_DPI_STAGES, hid_usage};

use crate::profile::{ButtonAction, Profile};

/// Polling rates a profile may ask for. Both protocol variants (period code and
/// HyperPolling bitmask) can encode each of them.
pub const POLLING_RATES: [u32; 3] = [125, 500, 1000];

/// Highest mouse-button number a `mouse-button` / `double-click` action may target
/// (protocol docs: `1` left … `8`; inferred).
pub const MAX_MOUSE_BUTTON: u8 = 8;

/// Device-specific limits, typically filled from the device registry entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceLimits {
    pub dpi_min: u16,
    pub dpi_max: u16,
    /// Valid DPI values are `dpi_min + k · dpi_step` (same rule as the registry).
    pub dpi_step: u16,
    /// Most DPI stages the device accepts (capped at the protocol maximum).
    pub max_dpi_stages: usize,
}

impl Default for DeviceLimits {
    /// No device knowledge: any non-zero DPI, as many stages as the protocol allows.
    fn default() -> Self {
        Self {
            dpi_min: 1,
            dpi_max: u16::MAX,
            dpi_step: 1,
            max_dpi_stages: MAX_DPI_STAGES,
        }
    }
}

impl DeviceLimits {
    pub fn dpi_valid(&self, dpi: u16) -> bool {
        let step = self.dpi_step.max(1);
        (self.dpi_min..=self.dpi_max).contains(&dpi) && (dpi - self.dpi_min).is_multiple_of(step)
    }
}

/// One problem in a profile, located by its TOML path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// TOML path, e.g. `dpi.stages[2]` or `hypershift.buttons.4.key`.
    pub path: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// All problems found in a profile (never empty).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct ValidationError(pub Vec<Issue>);

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "profile has {} problem(s)", self.0.len())?;
        for i in &self.0 {
            write!(f, "\n  {i}")?;
        }
        Ok(())
    }
}

impl Profile {
    /// Check every value against the protocol and `limits`. Reports all problems at once.
    pub fn validate(&self, limits: &DeviceLimits) -> Result<(), ValidationError> {
        let mut issues = Vec::new();
        let mut bad = |path: String, message: String| issues.push(Issue { path, message });

        if let Some(dpi) = &self.dpi {
            let max = limits.max_dpi_stages.min(MAX_DPI_STAGES);
            let n = dpi.stages.len();
            if n == 0 || n > max {
                bad(
                    "dpi.stages".into(),
                    format!("needs 1 to {max} stages, has {n}"),
                );
            }
            for (i, &v) in dpi.stages.iter().enumerate() {
                if !limits.dpi_valid(v) {
                    bad(
                        format!("dpi.stages[{i}]"),
                        format!(
                            "{v} DPI is outside {}-{} (step {})",
                            limits.dpi_min, limits.dpi_max, limits.dpi_step
                        ),
                    );
                }
            }
            if n > 0 && !(1..=n).contains(&usize::from(dpi.active_stage)) {
                bad(
                    "dpi.active_stage".into(),
                    format!("must be 1 to {n} (1-based), is {}", dpi.active_stage),
                );
            }
        }

        if let Some(p) = &self.polling
            && !POLLING_RATES.contains(&p.hz)
        {
            bad(
                "polling.hz".into(),
                format!("{} Hz is not one of {POLLING_RATES:?}", p.hz),
            );
        }

        for (layer, id, action) in self.all_buttons() {
            let base = match layer {
                Layer::Normal => format!("buttons.{id}"),
                Layer::HyperShift => format!("hypershift.buttons.{id}"),
            };
            if id.0 == 0 {
                bad(base.clone(), "button ID must be 1 or higher".into());
            }
            match action {
                ButtonAction::MouseButton { target } | ButtonAction::DoubleClick { target } => {
                    if !(1..=MAX_MOUSE_BUTTON).contains(target) {
                        bad(
                            format!("{base}.target"),
                            format!("mouse button must be 1 to {MAX_MOUSE_BUTTON}, is {target}"),
                        );
                    }
                }
                ButtonAction::Key { key, mods } => {
                    if hid_usage::usage(key).is_none() {
                        bad(
                            format!("{base}.key"),
                            format!(
                                "unknown key {key:?} (a-z, 0-9, f1-f12, enter, esc, backspace, tab, space)"
                            ),
                        );
                    }
                    let mut seen = 0u8;
                    for m in mods {
                        match hid_usage::modifier(m) {
                            None => bad(
                                format!("{base}.mods"),
                                format!("unknown modifier {m:?} (ctrl, shift, alt, win)"),
                            ),
                            Some(bit) if seen & bit != 0 => {
                                bad(format!("{base}.mods"), format!("modifier {m:?} repeated"))
                            }
                            Some(bit) => seen |= bit,
                        }
                    }
                }
                ButtonAction::DpiUp {} => {}
            }
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(ValidationError(issues))
        }
    }
}
