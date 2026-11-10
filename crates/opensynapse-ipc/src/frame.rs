//! Length-prefixed framing: `u32` LE body length, then the body.

use std::io::{self, Read, Write};

/// Largest body accepted (1 MiB).
pub const MAX_FRAME: usize = 1 << 20;

/// Write one frame (header and body in a single write, then flush).
/// `InvalidInput` when `body` exceeds [`MAX_FRAME`].
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("IPC frame of {} bytes exceeds {MAX_FRAME}", body.len()),
        ));
    }
    let mut buf = Vec::with_capacity(4 + body.len());
    // Fits: body.len() <= MAX_FRAME < u32::MAX.
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(body);
    w.write_all(&buf)?;
    w.flush()
}

/// Read one frame. `UnexpectedEof` on a clean close, `InvalidData` when the
/// announced length exceeds [`MAX_FRAME`].
pub fn read_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 4];
    r.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("IPC frame of {len} bytes exceeds {MAX_FRAME}"),
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(body)
}
