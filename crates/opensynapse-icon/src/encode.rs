//! File formats for the icon, written with std only:
//! - PNG (RFC 2083 / ISO 15948) with a zlib stream (RFC 1950) of one fixed-Huffman
//!   deflate block (RFC 1951). Rows use the "Sub" filter, so flat areas become runs
//!   of equal bytes, coded as matches at distance 1. Good enough for an icon.
//! - ICO: `ICONDIR` + `ICONDIRENTRY`s with PNG images (supported since Vista).
//! - Windows `.res` (the compiled resource format `rc.exe` writes): an empty
//!   leading entry, one `RT_ICON` per image, and `RT_GROUP_ICON` 1 listing them.

use crate::{ICO_SIZES, render};

// ---------- PNG ----------

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// LSB-first bit writer (deflate bit order).
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn put(&mut self, value: u32, count: u32) {
        for i in 0..count {
            self.acc |= ((value >> i) & 1) << self.n;
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }

    /// Huffman codes go MSB first.
    fn huff(&mut self, code: u32, len: u32) {
        for i in (0..len).rev() {
            self.put((code >> i) & 1, 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Fixed literal/length code (RFC 1951 3.2.6).
fn lit(bits: &mut Bits, v: u32) {
    match v {
        0..=143 => bits.huff(0x30 + v, 8),
        144..=255 => bits.huff(0x190 + v - 144, 9),
        256..=279 => bits.huff(v - 256, 7),
        _ => bits.huff(0xC0 + v - 280, 8),
    }
}

const LEN_BASE: [u32; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// A match of `len` (3..=258) at distance 1.
fn run(bits: &mut Bits, len: u32) {
    let i = LEN_BASE.iter().rposition(|&b| b <= len).expect("len >= 3");
    lit(bits, 257 + i as u32);
    bits.put(len - LEN_BASE[i], LEN_EXTRA[i]);
    bits.huff(0, 5); // distance code 0 = distance 1, no extra bits
}

/// zlib stream with one fixed-Huffman block: literals, plus runs of a repeated byte.
pub(crate) fn zlib(data: &[u8]) -> Vec<u8> {
    let mut bits = Bits {
        out: vec![0x78, 0x01],
        acc: 0,
        n: 0,
    };
    bits.put(1, 1); // BFINAL
    bits.put(1, 2); // BTYPE = fixed Huffman
    let mut i = 0;
    while i < data.len() {
        let mut len = 0;
        if i > 0 {
            while i + len < data.len() && len < 258 && data[i + len] == data[i - 1] {
                len += 1;
            }
        }
        if len >= 3 {
            run(&mut bits, len as u32);
            i += len;
        } else {
            lit(&mut bits, u32::from(data[i]));
            i += 1;
        }
    }
    lit(&mut bits, 256); // end of block
    let mut out = bits.finish();
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// PNG (8-bit RGBA) of `rgba`, which is `size × size` straight RGBA.
pub fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = size as usize * 4;
    assert_eq!(rgba.len(), stride * size as usize, "rgba size mismatch");
    let mut raw = Vec::with_capacity((stride + 1) * size as usize);
    for row in rgba.chunks(stride) {
        raw.push(1); // filter: Sub
        for (i, &b) in row.iter().enumerate() {
            raw.push(b.wrapping_sub(if i >= 4 { row[i - 4] } else { 0 }));
        }
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bit, RGBA, deflate, adaptive, no interlace
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

// ---------- ICO / RES ----------

fn images() -> Vec<(u32, Vec<u8>)> {
    ICO_SIZES.iter().map(|&s| (s, png(s, &render(s)))).collect()
}

/// `.ico` with one PNG entry per size in [`ICO_SIZES`].
pub fn ico() -> Vec<u8> {
    let imgs = images();
    let mut out = Vec::new();
    for v in [0u16, 1, imgs.len() as u16] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut offset = 6 + 16 * imgs.len() as u32;
    for (size, data) in &imgs {
        out.extend_from_slice(&[dim(*size), dim(*size), 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &imgs {
        out.extend_from_slice(data);
    }
    out
}

/// ICO/RES width or height byte: 0 means 256.
fn dim(size: u32) -> u8 {
    if size >= 256 { 0 } else { size as u8 }
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
/// MOVEABLE | PURE | DISCARDABLE, as `rc.exe` sets for icons.
const ICON_FLAGS: u16 = 0x1030;
/// LANG_NEUTRAL / SUBLANG_NEUTRAL.
const LANG_NEUTRAL: u16 = 0;

/// One `.res` entry with numeric type and name (32-byte header), data padded to 4.
fn res_entry(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
    out.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    for v in [0xFFFF, kind, 0xFFFF, name] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&LANG_NEUTRAL.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// A compiled Windows resource file: icon group 1 (the one Explorer shows for an
/// exe) with one `RT_ICON` (ids 1..) per size.
pub fn windows_res() -> Vec<u8> {
    let imgs = images();
    let mut out = Vec::new();
    // Leading empty entry that marks a 32-bit resource file.
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    for v in [0xFFFFu16, 0, 0xFFFF, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0; 16]);

    let mut group = Vec::new();
    for v in [0u16, 1, imgs.len() as u16] {
        group.extend_from_slice(&v.to_le_bytes());
    }
    for (i, (size, data)) in imgs.iter().enumerate() {
        let id = i as u16 + 1;
        res_entry(&mut out, RT_ICON, id, ICON_FLAGS & !0x0020, data);
        group.extend_from_slice(&[dim(*size), dim(*size), 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(data.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    res_entry(&mut out, RT_GROUP_ICON, 1, ICON_FLAGS, &group);
    out
}
