//! Keyboard key names ↔ USB-HID usage IDs (Keyboard/Keypad page 0x07) and the
//! boot-keyboard modifier bits. Public USB-HID spec values (HID Usage Tables,
//! chapter "Keyboard/Keypad Page"); only the subset the CLI offers.

/// Named keys outside the letter/digit/F-key ranges.
const NAMED: [(&str, u8); 5] = [
    ("enter", 0x28),
    ("esc", 0x29),
    ("backspace", 0x2A),
    ("tab", 0x2B),
    ("space", 0x2C),
];

/// Left-side modifier bits of the boot-keyboard modifier byte.
const MODIFIERS: [(&str, u8); 4] = [
    ("ctrl", 0x01),
    ("shift", 0x02),
    ("alt", 0x04),
    ("win", 0x08),
];

/// Usage ID for a key name (`a`–`z`, `0`–`9`, `enter`, `esc`, `backspace`, `tab`,
/// `space`, `f1`–`f12`; case-insensitive).
pub fn usage(name: &str) -> Option<u8> {
    let n = name.to_ascii_lowercase();
    let b = n.as_bytes();
    match b {
        [c @ b'a'..=b'z'] => return Some(0x04 + (c - b'a')),
        [b'0'] => return Some(0x27),
        [c @ b'1'..=b'9'] => return Some(0x1E + (c - b'1')),
        _ => {}
    }
    if let Some(f) = n.strip_prefix('f').and_then(|s| s.parse::<u8>().ok())
        && (1..=12).contains(&f)
        && !n.starts_with("f0")
    {
        return Some(0x3A + f - 1);
    }
    NAMED.iter().find(|(k, _)| *k == n).map(|&(_, u)| u)
}

/// Key name for a usage ID, inverse of [`usage`].
pub fn name(usage: u8) -> Option<String> {
    match usage {
        0x04..=0x1D => Some(char::from(b'a' + (usage - 0x04)).to_string()),
        0x1E..=0x26 => Some(char::from(b'1' + (usage - 0x1E)).to_string()),
        0x27 => Some("0".into()),
        0x3A..=0x45 => Some(format!("f{}", usage - 0x3A + 1)),
        _ => NAMED
            .iter()
            .find(|&&(_, u)| u == usage)
            .map(|(k, _)| (*k).to_string()),
    }
}

/// Modifier bit for `ctrl`, `shift`, `alt` or `win` (left side; case-insensitive).
pub fn modifier(name: &str) -> Option<u8> {
    let n = name.to_ascii_lowercase();
    MODIFIERS.iter().find(|(k, _)| *k == n).map(|&(_, b)| b)
}

/// Names of the bits set in `mask`, or `None` if it has bits [`modifier`] cannot name
/// (the right-side modifiers 0x10–0x80).
pub fn modifier_names(mask: u8) -> Option<Vec<&'static str>> {
    if mask & 0xF0 != 0 {
        return None;
    }
    Some(
        MODIFIERS
            .iter()
            .filter(|&&(_, b)| mask & b != 0)
            .map(|&(k, _)| k)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_values() {
        assert_eq!(usage("a"), Some(0x04));
        assert_eq!(usage("Z"), Some(0x1D));
        assert_eq!(usage("1"), Some(0x1E));
        assert_eq!(usage("9"), Some(0x26));
        assert_eq!(usage("0"), Some(0x27));
        assert_eq!(usage("enter"), Some(0x28));
        assert_eq!(usage("esc"), Some(0x29));
        assert_eq!(usage("backspace"), Some(0x2A));
        assert_eq!(usage("tab"), Some(0x2B));
        assert_eq!(usage("space"), Some(0x2C));
        assert_eq!(usage("F1"), Some(0x3A));
        assert_eq!(usage("f12"), Some(0x45));
    }

    #[test]
    fn unknown_names() {
        for n in ["", "aa", "f0", "f13", "f01", "ctrl", "-", "10"] {
            assert_eq!(usage(n), None, "{n}");
        }
        assert_eq!(name(0x00), None);
        assert_eq!(name(0x46), None);
    }

    #[test]
    fn name_inverts_usage() {
        for u in (0x04..=0x2C).chain(0x3A..=0x45) {
            let n = name(u).unwrap();
            assert_eq!(usage(&n), Some(u), "{n}");
        }
    }

    #[test]
    fn modifiers() {
        assert_eq!(modifier("ctrl"), Some(0x01));
        assert_eq!(modifier("Shift"), Some(0x02));
        assert_eq!(modifier("alt"), Some(0x04));
        assert_eq!(modifier("win"), Some(0x08));
        assert_eq!(modifier("meta"), None);
        assert_eq!(modifier_names(0x00), Some(vec![]));
        assert_eq!(modifier_names(0x03), Some(vec!["ctrl", "shift"]));
        assert_eq!(
            modifier_names(0x0F),
            Some(vec!["ctrl", "shift", "alt", "win"])
        );
        assert_eq!(modifier_names(0x10), None);
    }
}
