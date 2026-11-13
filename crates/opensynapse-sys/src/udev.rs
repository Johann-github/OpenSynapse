//! Linux udev rule that lets the logged-in user open Razer hidraw nodes without
//! root (`TAG+="uaccess"`, systemd-logind). The file lives in
//! `data/udev/70-opensynapse.rules`; this is its one owner in code. Plain text,
//! available on every OS so `opensynapse udev-rule` can print it anywhere.
//! Plan: `vault/Architecture/Linux and macOS port.md` (stage 3).

/// File name to install the rule under.
pub const RULE_FILE_NAME: &str = "70-opensynapse.rules";

/// Folder the rule goes into on the target system.
pub const RULES_DIR: &str = "/etc/udev/rules.d";

/// Text of the rule file.
pub const UDEV_RULE: &str = include_str!("../../../data/udev/70-opensynapse.rules");

/// Shell commands that install the rule from a file saved as [`RULE_FILE_NAME`]
/// in the current directory and make udev apply it.
pub fn install_hint() -> String {
    format!(
        "sudo cp {RULE_FILE_NAME} {RULES_DIR}/\n\
         sudo udevadm control --reload-rules && sudo udevadm trigger\n\
         Then unplug and replug the device (or its receiver)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rule lines only (comments and blank lines dropped).
    fn rules() -> Vec<&'static str> {
        UDEV_RULE
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    }

    #[test]
    fn rule_grants_uaccess_on_razer_hidraw() {
        let r = rules();
        assert!(!r.is_empty());
        for line in &r {
            assert!(line.contains(r#"SUBSYSTEM=="hidraw""#), "{line}");
            assert!(line.contains("1532"), "{line}");
            assert!(line.contains(r#"TAG+="uaccess""#), "{line}");
        }
        assert!(r.iter().any(|l| l.contains(r#"ATTRS{idVendor}=="1532""#)));
        assert!(r.iter().any(|l| l.contains(r#"KERNELS=="0005:1532:*""#)));
    }

    #[test]
    fn rule_sets_no_open_mode() {
        assert!(!UDEV_RULE.contains("0666"));
        assert!(!UDEV_RULE.contains("MODE="));
    }

    #[test]
    fn hint_names_file_and_reload() {
        let h = install_hint();
        assert!(h.contains("/etc/udev/rules.d/"));
        assert!(h.contains(RULE_FILE_NAME));
        assert!(h.contains("udevadm control --reload-rules"));
        assert!(h.contains("udevadm trigger"));
    }
}
