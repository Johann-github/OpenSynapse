//! Start-menu shortcut for the GUI, written as a `.lnk` file without COM
//! (`vault/Decisions/ADR-0009 Install location.md`).
//!
//! Layout from Microsoft's open specification "[MS-SHLLINK]: Shell Link (.LNK)
//! Binary File Format": a 76-byte `ShellLinkHeader`, a `LinkInfo` block that names
//! the target by local path (ANSI and Unicode), `StringData` (description and
//! working folder, UTF-16) and the 4-byte terminal block. No `LinkTargetIDList`:
//! the shell resolves the target from `LinkInfo`. [`encode`] is pure; only
//! [`start_menu_dir`] reads the environment.

use std::path::PathBuf;

/// File name of the shortcut in the Start-menu programs folder.
pub const FILE_NAME: &str = "OpenSynapse.lnk";

/// `ShellLinkHeader.HeaderSize`.
const HEADER_SIZE: u32 = 0x4C;
/// `ShellLinkHeader.LinkCLSID` = {00021401-0000-0000-C000-000000000046}, as stored.
const LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
/// `LinkFlags`: HasLinkInfo | HasName | HasWorkingDir | HasIconLocation | IsUnicode |
/// HasExpString.
const LINK_FLAGS: u32 = 0x02 | 0x04 | 0x10 | 0x40 | 0x80 | 0x200;
/// `EnvironmentVariableDataBlock`: size and signature; ANSI and Unicode target
/// fields of 260 characters each.
const ENV_BLOCK_SIZE: u32 = 0x314;
const ENV_BLOCK_SIGNATURE: u32 = 0xA000_0001;
const MAX_PATH: usize = 260;
/// `FileAttributes`: FILE_ATTRIBUTE_ARCHIVE (an ordinary file).
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
/// `ShowCommand`: SW_SHOWNORMAL.
const SW_SHOWNORMAL: u32 = 1;
/// `LinkInfoHeaderSize` with the two Unicode offsets present.
const LINK_INFO_HEADER_SIZE: u32 = 0x24;
/// `LinkInfoFlags`: VolumeIDAndLocalBasePath.
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 1;
/// `VolumeID.DriveType`: DRIVE_FIXED.
const DRIVE_FIXED: u32 = 3;
/// `VolumeID` header size (size, type, serial, label offset); the label follows.
const VOLUME_ID_HEADER: u32 = 0x10;

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs` for an `APPDATA` value.
pub fn start_menu_dir_from(appdata: Option<&str>) -> Option<PathBuf> {
    let root = appdata.filter(|d| !d.trim().is_empty())?;
    Some(
        PathBuf::from(root)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs"),
    )
}

/// The current user's Start-menu programs folder (Windows only).
pub fn start_menu_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        start_menu_dir_from(std::env::var("APPDATA").ok().as_deref())
    } else {
        None
    }
}

/// The `.lnk` bytes for a shortcut to `target` (absolute path) that starts in
/// `working_dir` and shows `description` as its comment.
pub fn encode(target: &str, working_dir: &str, description: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    header(&mut out);
    link_info(&mut out, target);
    string_data(&mut out, description);
    string_data(&mut out, working_dir);
    // Icon: the target's own (index 0, set in the header).
    string_data(&mut out, target);
    // ExtraData: the target again as an environment-variable path (the shell
    // resolves a shortcut without an ID list through it; observed 2026-11-11:
    // with LinkInfo alone `TargetPath` stayed empty), then the TerminalBlock.
    env_block(&mut out, target);
    put_u32(&mut out, 0);
    out
}

fn env_block(out: &mut Vec<u8>, target: &str) {
    put_u32(out, ENV_BLOCK_SIZE);
    put_u32(out, ENV_BLOCK_SIGNATURE);
    let mut ansi = [0u8; MAX_PATH];
    for (slot, c) in ansi.iter_mut().zip(target.chars().take(MAX_PATH - 1)) {
        *slot = if c.is_ascii() { c as u8 } else { b'?' };
    }
    out.extend_from_slice(&ansi);
    let mut uni = [0u16; MAX_PATH];
    for (slot, u) in uni.iter_mut().zip(target.encode_utf16().take(MAX_PATH - 1)) {
        *slot = u;
    }
    for u in uni {
        put_u16(out, u);
    }
}

fn header(out: &mut Vec<u8>) {
    put_u32(out, HEADER_SIZE);
    out.extend_from_slice(&LINK_CLSID);
    put_u32(out, LINK_FLAGS);
    put_u32(out, FILE_ATTRIBUTE_ARCHIVE);
    out.extend_from_slice(&[0; 24]); // creation, access, write time
    put_u32(out, 0); // FileSize
    put_u32(out, 0); // IconIndex
    put_u32(out, SW_SHOWNORMAL);
    put_u16(out, 0); // HotKey
    put_u16(out, 0); // Reserved1
    put_u32(out, 0); // Reserved2
    put_u32(out, 0); // Reserved3
}

fn link_info(out: &mut Vec<u8>, target: &str) {
    // VolumeID with an empty ANSI label.
    let volume_size = VOLUME_ID_HEADER + 1;
    // ANSI copy of the path: non-ASCII characters become '?'; the shell uses the
    // Unicode copy, which this header size announces.
    let ansi: Vec<u8> = target
        .chars()
        .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
        .chain([0])
        .collect();
    let unicode = utf16_nul(target);

    let volume_off = LINK_INFO_HEADER_SIZE;
    let base_off = volume_off + volume_size;
    let suffix_off = base_off + ansi.len() as u32;
    let base_uni_off = suffix_off + 1;
    let suffix_uni_off = base_uni_off + unicode.len() as u32;
    let size = suffix_uni_off + 2;

    put_u32(out, size);
    put_u32(out, LINK_INFO_HEADER_SIZE);
    put_u32(out, VOLUME_ID_AND_LOCAL_BASE_PATH);
    put_u32(out, volume_off);
    put_u32(out, base_off);
    put_u32(out, 0); // CommonNetworkRelativeLinkOffset
    put_u32(out, suffix_off);
    put_u32(out, base_uni_off);
    put_u32(out, suffix_uni_off);

    put_u32(out, volume_size);
    put_u32(out, DRIVE_FIXED);
    put_u32(out, 0); // DriveSerialNumber
    put_u32(out, VOLUME_ID_HEADER); // VolumeLabelOffset
    out.push(0); // empty label

    out.extend_from_slice(&ansi);
    out.push(0); // empty CommonPathSuffix (ANSI)
    out.extend_from_slice(&unicode);
    put_u16(out, 0); // empty CommonPathSuffix (Unicode)
}

/// One `StringData` entry: character count, then UTF-16 without terminator.
fn string_data(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    put_u16(out, units.len() as u16);
    for u in units {
        put_u16(out, u);
    }
}

fn utf16_nul(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn u16_at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
    }

    #[test]
    fn header_fields() {
        let b = encode(r"C:\a\gui.exe", r"C:\a", "x");
        assert_eq!(u32_at(&b, 0), 0x4C);
        assert_eq!(&b[4..20], &LINK_CLSID);
        assert_eq!(u32_at(&b, 20), 0x2D6);
        assert_eq!(u32_at(&b, 60), 1, "ShowCommand");
        assert_eq!(&b[b.len() - 4..], &[0, 0, 0, 0], "terminal block");
    }

    #[test]
    fn link_info_offsets_point_at_the_paths() {
        let target = r"C:\Users\ü\OpenSynapse\opensynapse-desktop.exe";
        let b = encode(target, r"C:\Users\ü\OpenSynapse", "OpenSynapse");
        let li = 76;
        let size = u32_at(&b, li) as usize;
        assert_eq!(u32_at(&b, li + 4), 0x24);
        assert_eq!(u32_at(&b, li + 8), 1);
        // VolumeID: fixed drive, empty label.
        let vol = li + u32_at(&b, li + 12) as usize;
        assert_eq!(u32_at(&b, vol + 4), DRIVE_FIXED);
        assert_eq!(b[vol + u32_at(&b, vol + 12) as usize], 0);
        // ANSI path with '?' for the non-ASCII character, NUL-terminated.
        let ansi = li + u32_at(&b, li + 16) as usize;
        let end = b[ansi..].iter().position(|&c| c == 0).unwrap();
        assert_eq!(
            std::str::from_utf8(&b[ansi..ansi + end]).unwrap(),
            r"C:\Users\?\OpenSynapse\opensynapse-desktop.exe"
        );
        // Unicode path, NUL-terminated.
        let uni = li + u32_at(&b, li + 28) as usize;
        let units: Vec<u16> = (0..)
            .map(|i| u16_at(&b, uni + 2 * i))
            .take_while(|&u| u != 0)
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), target);
        // Both suffixes empty, and the block ends where its size says.
        assert_eq!(b[li + u32_at(&b, li + 24) as usize], 0);
        let suffix_uni = li + u32_at(&b, li + 32) as usize;
        assert_eq!(u16_at(&b, suffix_uni), 0);
        assert_eq!(suffix_uni + 2, li + size);
        // StringData follows: description, then working folder.
        let mut at = li + size;
        let n = u16_at(&b, at) as usize;
        assert_eq!(n, "OpenSynapse".len());
        at += 2 + 2 * n;
        let n = u16_at(&b, at) as usize;
        let dir: Vec<u16> = (0..n).map(|i| u16_at(&b, at + 2 + 2 * i)).collect();
        assert_eq!(String::from_utf16(&dir).unwrap(), r"C:\Users\ü\OpenSynapse");
        // Icon location = the target.
        at += 2 + 2 * n;
        let n = u16_at(&b, at) as usize;
        let icon: Vec<u16> = (0..n).map(|i| u16_at(&b, at + 2 + 2 * i)).collect();
        assert_eq!(String::from_utf16(&icon).unwrap(), target);
        // Environment-variable block with the Unicode target, then the terminal block.
        at += 2 + 2 * n;
        assert_eq!(u32_at(&b, at), ENV_BLOCK_SIZE);
        assert_eq!(u32_at(&b, at + 4), ENV_BLOCK_SIGNATURE);
        let uni = at + 8 + MAX_PATH;
        let units: Vec<u16> = (0..)
            .map(|i| u16_at(&b, uni + 2 * i))
            .take_while(|&u| u != 0)
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), target);
        assert_eq!(at + ENV_BLOCK_SIZE as usize + 4, b.len());
    }

    #[test]
    fn start_menu_folder() {
        assert_eq!(
            start_menu_dir_from(Some(r"C:\Users\u\AppData\Roaming")),
            Some(PathBuf::from(
                r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs"
            ))
        );
        assert_eq!(start_menu_dir_from(Some(" ")), None);
        assert_eq!(start_menu_dir_from(None), None);
    }
}

/// Manual check against the Windows shell: writes a shortcut to
/// `$OPENSYNAPSE_LNK_OUT` (`cargo test -p opensynapse-sys write_sample -- --ignored`),
/// then read it with `(New-Object -ComObject WScript.Shell).CreateShortcut(path)`.
#[cfg(test)]
#[test]
#[ignore]
fn write_sample() {
    let out = std::env::var("OPENSYNAPSE_LNK_OUT").expect("set OPENSYNAPSE_LNK_OUT");
    let target = std::env::var("OPENSYNAPSE_LNK_TARGET").expect("set OPENSYNAPSE_LNK_TARGET");
    let dir = std::path::Path::new(&target).parent().unwrap();
    std::fs::write(out, encode(&target, &dir.to_string_lossy(), "OpenSynapse")).unwrap();
}
