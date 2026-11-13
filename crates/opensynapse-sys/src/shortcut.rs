//! Start-menu shortcut for the GUI, written as a `.lnk` file without COM
//! (`vault/Decisions/ADR-0009 Install location.md`).
//!
//! Layout from Microsoft's open specification "[MS-SHLLINK]: Shell Link (.LNK)
//! Binary File Format": a 76-byte `ShellLinkHeader`, a `LinkTargetIDList` (for a
//! drive path: "This PC" → drive → one file-entry item per component, so the
//! Properties dialog shows the target), a `LinkInfo` block that names the target
//! by local path (ANSI and Unicode), `StringData` (description, working folder,
//! icon, UTF-16), an environment-variable block and the 4-byte terminal block.
//! The shell-item layout (file entry + `0xBEEF0004` extension, version 3) follows
//! the public libfwsi documentation of Windows shell items. [`encode`] is pure;
//! only [`start_menu_dir`] reads the environment.

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
/// `LinkFlags.HasLinkTargetIDList`.
const HAS_ID_LIST: u32 = 0x01;
/// Root shell item: "This PC" (CLSID {20D04FE0-3AEA-1069-A2D8-08002B30309D}).
const ROOT_MY_COMPUTER: [u8; 20] = [
    0x14, 0x00, 0x1F, 0x50, 0xE0, 0x4F, 0xD0, 0x20, 0xEA, 0x3A, 0x69, 0x10, 0xA2, 0xD8, 0x08, 0x00,
    0x2B, 0x30, 0x30, 0x9D,
];
/// Volume shell item: class type 0x2F, then the drive ("C:\") padded to 22 bytes.
const DRIVE_ITEM_TYPE: u8 = 0x2F;
const DRIVE_ITEM_DATA: usize = 22;
/// File-entry shell item class types.
const ITEM_DIRECTORY: u8 = 0x31;
const ITEM_FILE: u8 = 0x32;
/// `FILE_ATTRIBUTE_DIRECTORY`.
const FILE_ATTRIBUTE_DIRECTORY: u16 = 0x10;
/// File-entry extension block: signature, version 3, and the offset of the long
/// name inside the block (stored at +16; observed: a shell-made shortcut has
/// 0x2E there for version 9 with its name at +0x2E).
const EXT_SIGNATURE: u32 = 0xBEEF_0004;
const EXT_VERSION: u16 = 3;
const EXT_V3_NAME_OFFSET: u16 = 0x14;
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
    let mut out = Vec::with_capacity(1024);
    let ids = id_list(target);
    header(&mut out, ids.is_some());
    if let Some(ids) = ids {
        put_u16(&mut out, ids.len() as u16);
        out.extend_from_slice(&ids);
    }
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

/// The `IDList` (items + 2-byte terminator) for an absolute drive path such as
/// `C:\dir\app.exe`; `None` for anything else (UNC, relative), which then relies
/// on `LinkInfo` alone. Directories are all components but the last.
fn id_list(target: &str) -> Option<Vec<u8>> {
    let b = target.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return None;
    }
    let parts: Vec<&str> = target[3..].split('\\').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() || parts.len() > 64 {
        return None;
    }
    let mut out = ROOT_MY_COMPUTER.to_vec();
    // Volume item: size, type, "C:\" + zero padding.
    put_u16(&mut out, (3 + DRIVE_ITEM_DATA) as u16);
    out.push(DRIVE_ITEM_TYPE);
    let mut drive = [0u8; DRIVE_ITEM_DATA];
    drive[..3].copy_from_slice(&[b[0].to_ascii_uppercase(), b':', b'\\']);
    out.extend_from_slice(&drive);
    for (i, name) in parts.iter().enumerate() {
        let dir = i + 1 < parts.len();
        out.extend_from_slice(&file_entry(name, dir));
    }
    put_u16(&mut out, 0); // TerminalID
    Some(out)
}

/// One file-entry shell item: size, class type, size/date/attributes left 0
/// (the shell reads the live file), the name as primary (ANSI, `?` for
/// non-ASCII) and as Unicode long name in a version-3 `0xBEEF0004` extension.
fn file_entry(name: &str, dir: bool) -> Vec<u8> {
    let mut item = vec![0, 0]; // size, patched below
    item.push(if dir { ITEM_DIRECTORY } else { ITEM_FILE });
    item.push(0);
    put_u32(&mut item, 0); // file size
    put_u32(&mut item, 0); // modification date/time (FAT)
    put_u16(&mut item, if dir { FILE_ATTRIBUTE_DIRECTORY } else { 0x20 });
    item.extend(
        name.chars()
            .map(|c| if c.is_ascii() { c as u8 } else { b'?' }),
    );
    item.push(0);
    if item.len() % 2 == 1 {
        item.push(0);
    }
    let ext_off = item.len();
    let long = utf16_nul(name);
    let ext_size = usize::from(EXT_V3_NAME_OFFSET) + long.len() + 2;
    put_u16(&mut item, ext_size as u16);
    put_u16(&mut item, EXT_VERSION);
    put_u32(&mut item, EXT_SIGNATURE);
    put_u32(&mut item, 0); // creation date/time
    put_u32(&mut item, 0); // last access date/time
    put_u16(&mut item, EXT_V3_NAME_OFFSET);
    put_u16(&mut item, 0); // localized-name size: none
    item.extend_from_slice(&long);
    put_u16(&mut item, ext_off as u16); // offset of this extension in the item
    let size = item.len() as u16;
    item[..2].copy_from_slice(&size.to_le_bytes());
    item
}

fn header(out: &mut Vec<u8>, has_ids: bool) {
    put_u32(out, HEADER_SIZE);
    out.extend_from_slice(&LINK_CLSID);
    put_u32(out, LINK_FLAGS | if has_ids { HAS_ID_LIST } else { 0 });
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
        assert_eq!(u32_at(&b, 20), 0x2D7, "flags incl. HasLinkTargetIDList");
        assert_eq!(u32_at(&b, 60), 1, "ShowCommand");
        assert_eq!(&b[b.len() - 4..], &[0, 0, 0, 0], "terminal block");
        // No drive letter: no ID list, LinkInfo follows the header directly.
        let unc = encode(r"\\host\share\gui.exe", r"\\host\share", "x");
        assert_eq!(u32_at(&unc, 20), 0x2D6);
        assert_eq!(u32_at(&unc, 80), 0x24, "LinkInfoHeaderSize at 76 + 4");
    }

    /// Walk the ID list: root, drive, then one file entry per component whose
    /// primary name and Unicode long name match the path.
    #[test]
    fn id_list_items_name_each_component() {
        let target = r"c:\Users\ü\app.exe";
        let b = encode(target, r"c:\Users\ü", "x");
        let total = u16_at(&b, 76) as usize;
        let ids = &b[78..78 + total];
        assert_eq!(&ids[..20], &ROOT_MY_COMPUTER);
        assert_eq!(u16_at(ids, 20), 25);
        assert_eq!(&ids[22..26], b"\x2FC:\\", "drive upper-cased");
        let mut at = 45;
        let mut seen = Vec::new();
        loop {
            let size = u16_at(ids, at) as usize;
            if size == 0 {
                assert_eq!(at + 2, total, "terminator ends the list");
                break;
            }
            let item = &ids[at..at + size];
            let ext = u16_at(item, size - 2) as usize;
            assert_eq!(u16_at(item, ext), (size - ext) as u16, "extension size");
            assert_eq!(u16_at(item, ext + 2), EXT_VERSION);
            assert_eq!(u32_at(item, ext + 4), EXT_SIGNATURE);
            let units: Vec<u16> = (0..)
                .map(|i| u16_at(item, ext + u16_at(item, ext + 16) as usize + 2 * i))
                .take_while(|&u| u != 0)
                .collect();
            let primary_end = 14 + item[14..].iter().position(|&c| c == 0).unwrap();
            seen.push((
                item[2],
                String::from_utf8(item[14..primary_end].to_vec()).unwrap(),
                String::from_utf16(&units).unwrap(),
            ));
            at += size;
        }
        assert_eq!(
            seen,
            [
                (ITEM_DIRECTORY, "Users".into(), "Users".into()),
                (ITEM_DIRECTORY, "?".into(), "ü".into()),
                (ITEM_FILE, "app.exe".into(), "app.exe".into()),
            ]
        );
    }

    #[test]
    fn link_info_offsets_point_at_the_paths() {
        let target = r"C:\Users\ü\OpenSynapse\opensynapse-desktop.exe";
        let b = encode(target, r"C:\Users\ü\OpenSynapse", "OpenSynapse");
        let li = 78 + u16_at(&b, 76) as usize;
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
