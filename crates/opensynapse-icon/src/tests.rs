use super::*;
use crate::encode::zlib;

/// Minimal inflate for fixed-Huffman blocks with distance-1 matches, enough to
/// round-trip what [`zlib`] writes.
fn inflate_fixed(z: &[u8]) -> Vec<u8> {
    let data = &z[2..z.len() - 4];
    let mut pos = 0usize;
    let mut bit = |n: u32| -> u32 {
        let mut v = 0;
        for i in 0..n {
            let b = (data[pos / 8] >> (pos % 8)) & 1;
            v |= u32::from(b) << i;
            pos += 1;
        }
        v
    };
    assert_eq!(bit(1), 1, "final block");
    assert_eq!(bit(2), 1, "fixed Huffman");
    let mut out = Vec::new();
    loop {
        // Read a fixed literal/length code MSB first.
        let mut code = 0u32;
        let mut len = 0;
        let sym = loop {
            code = (code << 1) | bit(1);
            len += 1;
            match len {
                7 if code <= 0x17 => break code + 256,
                8 if (0x30..=0xBF).contains(&code) => break code - 0x30,
                8 if (0xC0..=0xC7).contains(&code) => break code - 0xC0 + 280,
                9 if (0x190..=0x1FF).contains(&code) => break code - 0x190 + 144,
                9 => panic!("bad code"),
                _ => {}
            }
        };
        match sym {
            0..=255 => out.push(sym as u8),
            256 => break,
            _ => {
                let i = (sym - 257) as usize;
                let base = [
                    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67,
                    83, 99, 115, 131, 163, 195, 227, 258,
                ][i];
                let extra = [
                    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5,
                    5, 5, 0,
                ][i];
                let n = base + bit(extra);
                let mut d = 0;
                for _ in 0..5 {
                    d = (d << 1) | bit(1);
                }
                assert_eq!(d, 0, "distance 1 only");
                let last = *out.last().unwrap();
                out.extend(std::iter::repeat_n(last, n as usize));
            }
        }
    }
    out
}

#[test]
fn zlib_round_trips() {
    let mut data: Vec<u8> = (0..=255u8).collect();
    data.extend(std::iter::repeat_n(7u8, 1000));
    data.extend([1, 2, 3, 3, 3, 3, 9]);
    assert_eq!(inflate_fixed(&zlib(&data)), data);
    assert_eq!(inflate_fixed(&zlib(&[])), Vec::<u8>::new());
}

#[test]
fn render_has_transparent_corners_and_opaque_centre() {
    for s in ICO_SIZES {
        let px = render(s);
        assert_eq!(px.len(), (s * s * 4) as usize);
        assert_eq!(px[3], 0, "top-left corner transparent at {s}");
        let c = ((s / 2) * s + s / 2) as usize * 4;
        assert_eq!(px[c + 3], 255, "centre opaque at {s}");
    }
}

#[test]
fn png_layout() {
    let p = png(16, &render(16));
    assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&p[12..16], b"IHDR");
    assert_eq!(u32::from_be_bytes(p[16..20].try_into().unwrap()), 16);
    assert_eq!(&p[p.len() - 8..p.len() - 4], b"IEND");
}

#[test]
fn ico_directory_points_at_pngs() {
    let i = ico();
    assert_eq!(&i[..6], &[0, 0, 1, 0, ICO_SIZES.len() as u8, 0]);
    for k in 0..ICO_SIZES.len() {
        let e = 6 + 16 * k;
        let len = u32::from_le_bytes(i[e + 8..e + 12].try_into().unwrap()) as usize;
        let off = u32::from_le_bytes(i[e + 12..e + 16].try_into().unwrap()) as usize;
        assert_eq!(&i[off..off + 8], b"\x89PNG\r\n\x1a\n");
        assert!(off + len <= i.len());
    }
    assert_eq!(i[6 + 16 * (ICO_SIZES.len() - 1)], 0, "256 is stored as 0");
}

#[test]
fn res_entries_are_aligned_and_end_with_the_group() {
    let r = windows_res();
    let mut at = 0;
    let mut kinds = Vec::new();
    while at < r.len() {
        assert_eq!(at % 4, 0);
        let size = u32::from_le_bytes(r[at..at + 4].try_into().unwrap()) as usize;
        let header = u32::from_le_bytes(r[at + 4..at + 8].try_into().unwrap()) as usize;
        kinds.push(u16::from_le_bytes(r[at + 10..at + 12].try_into().unwrap()));
        at += (header + size).div_ceil(4) * 4;
    }
    assert_eq!(at, r.len());
    assert_eq!(kinds[0], 0, "leading empty entry");
    assert_eq!(kinds.len(), ICO_SIZES.len() + 2);
    assert_eq!(*kinds.last().unwrap(), 14, "RT_GROUP_ICON last");
}

/// Writes `opensynapse.ico` and a 256 px PNG to `$OPENSYNAPSE_ICON_OUT` for a look:
/// `cargo test -p opensynapse-icon write_preview -- --ignored`.
#[test]
#[ignore]
fn write_preview() {
    let dir = std::path::PathBuf::from(std::env::var("OPENSYNAPSE_ICON_OUT").expect("set it"));
    std::fs::write(dir.join("opensynapse.ico"), ico()).unwrap();
    std::fs::write(dir.join("opensynapse-256.png"), png(256, &render(256))).unwrap();
    std::fs::write(dir.join("opensynapse-32.png"), png(32, &render(32))).unwrap();
    std::fs::write(dir.join("opensynapse-16.png"), png(16, &render(16))).unwrap();
}
