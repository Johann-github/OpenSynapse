//! Length-prefixed framing: `u32` LE body length, then the body.

use std::io::{self, Read, Write};

/// Largest body accepted (1 MiB).
pub const MAX_FRAME: usize = 1 << 20;

/// Write one frame.
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let _ = (w, body);
    todo!("ipc frame: write_frame")
}

/// Read one frame. `UnexpectedEof` on a clean close, `InvalidData` when the
/// announced length exceeds [`MAX_FRAME`].
pub fn read_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let _ = r;
    todo!("ipc frame: read_frame")
}
