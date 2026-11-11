//! Per-app profile rules (`app-profiles.toml`): which profile file the agent
//! applies while an application is in the foreground. Pure: text in, rules out
//! (and rules in, text out for the GUI editor); path handling is string/`Path`
//! arithmetic only, no file system access.
//! Decision: `vault/Decisions/ADR-0007 Per-app profiles.md`.
//!
//! ```toml
//! # profile paths are relative to this file's folder, or absolute
//! default = "profiles/desktop.toml"   # optional: applied when no rule matches
//! [[app]]
//! exe = "cs2.exe"                      # file name (any case) or a full path
//! profile = "profiles/cs2.toml"
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// First lines of a file written by [`AppProfiles::to_toml`].
const HEADER: &str = "# OpenSynapse per-app profiles. Profile paths are relative to this file's\n\
                      # folder, or absolute. The first matching [[app]] rule wins.\n";

/// The parsed rules file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppProfiles {
    /// Profile applied when no rule matches the foreground application.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Rules in file order; the first match wins.
    #[serde(default, rename = "app", skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<AppRule>,
}

/// One `[[app]]` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppRule {
    /// Executable file name (`cs2.exe`) or full path; compared without case.
    pub exe: String,
    /// Profile TOML file, relative to the rules file's folder or absolute.
    pub profile: String,
}

/// Why a rules file was rejected.
#[derive(Debug, thiserror::Error)]
pub enum AppProfilesError {
    #[error("invalid app-profiles file: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("app-profiles rule {index}: {problem}")]
    Rule { index: usize, problem: &'static str },
    #[error("app-profiles `default` must not be empty")]
    EmptyDefault,
    #[error("cannot write app-profiles file: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl AppProfiles {
    /// Parse and check the rules (see [`Self::validate`]).
    pub fn from_toml(text: &str) -> Result<Self, AppProfilesError> {
        let p: Self = toml::from_str(text)?;
        p.validate()?;
        Ok(p)
    }

    /// Check the rules, then write them as TOML with a short header comment.
    /// [`Self::from_toml`] reads the result back unchanged. Comments of a file
    /// read earlier are not kept.
    pub fn to_toml(&self) -> Result<String, AppProfilesError> {
        self.validate()?;
        Ok(format!("{HEADER}{}", toml::to_string(self)?))
    }

    /// Non-empty `exe` and `profile` in every rule (`index` is 1-based), and a
    /// non-empty `default` if one is set.
    pub fn validate(&self) -> Result<(), AppProfilesError> {
        for (i, r) in self.apps.iter().enumerate() {
            let problem = if r.exe.trim().is_empty() {
                "`exe` must not be empty"
            } else if r.profile.trim().is_empty() {
                "`profile` must not be empty"
            } else {
                continue;
            };
            return Err(AppProfilesError::Rule {
                index: i + 1,
                problem,
            });
        }
        if self.default.as_deref().is_some_and(|d| d.trim().is_empty()) {
            return Err(AppProfilesError::EmptyDefault);
        }
        Ok(())
    }

    /// Rules that can never match because an earlier rule has the same `exe`
    /// pattern (same normalisation as [`exe_matches`]): `(rule, earlier rule)`,
    /// both 0-based.
    pub fn shadowed(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (i, r) in self.apps.iter().enumerate() {
            let me = norm_exe(&r.exe);
            if let Some(j) = self.apps[..i].iter().position(|e| norm_exe(&e.exe) == me) {
                out.push((i, j));
            }
        }
        out
    }

    /// The first rule matching `exe` (a full image path as Windows reports it).
    pub fn rule_for(&self, exe: &str) -> Option<&AppRule> {
        self.apps.iter().find(|r| exe_matches(&r.exe, exe))
    }

    /// The profile for `exe`: its rule's, else `default`, else `None`.
    pub fn profile_for(&self, exe: &str) -> Option<&str> {
        self.rule_for(exe)
            .map(|r| r.profile.as_str())
            .or(self.default.as_deref())
    }
}

/// Trimmed, lower case, `/` as `\`.
fn norm_exe(s: &str) -> String {
    s.trim().replace('/', "\\").to_lowercase()
}

/// `pattern` with a path separator is compared against the whole path, otherwise
/// against the file name; both without case, `/` and `\` alike.
pub fn exe_matches(pattern: &str, exe: &str) -> bool {
    let pattern = norm_exe(pattern);
    let exe = norm_exe(exe);
    if pattern.contains('\\') {
        pattern == exe
    } else {
        exe.rsplit('\\').next() == Some(pattern.as_str())
    }
}

/// A profile path from the rules file, resolved against the folder of
/// `rules_file` unless it is absolute.
pub fn resolve_profile_path(rules_file: &Path, profile: &str) -> PathBuf {
    let p = Path::new(profile);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    rules_file.parent().unwrap_or(Path::new("")).join(p)
}

#[cfg(test)]
mod tests;
