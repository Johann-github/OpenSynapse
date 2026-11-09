//! Detect Razer software that talks to the same device. Both apps can open the
//! control collection at once and interleave reports (`vault/Protocol/Windows Transport.md`).

/// Process image names that drive Razer devices.
const CONFLICTING: &[&str] = &["RazerAppEngine.exe"];

/// Name of a running conflicting process, if any. Spawns `tasklist` once
/// (Windows only); call it only before talking to a device, not for `list`.
#[cfg(windows)]
pub fn conflicting_process() -> Option<&'static str> {
    CONFLICTING.iter().copied().find(|name| {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}"), "/NH", "/FO", "CSV"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .to_ascii_lowercase()
                    .contains(&name.to_ascii_lowercase())
            })
            .unwrap_or(false)
    })
}

#[cfg(not(windows))]
pub fn conflicting_process() -> Option<&'static str> {
    let _ = CONFLICTING;
    None
}
