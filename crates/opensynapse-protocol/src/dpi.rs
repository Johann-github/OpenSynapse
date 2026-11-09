//! DPI and DPI stages (class 0x04). `vault/Protocol/Commands 2.5.md`.
//!
//! All 16-bit values are big-endian (observed).

use crate::command::*;
use crate::{ProtocolError, Request, be16, fixed, need};

/// Byte 0 of every DPI command.
///
/// Meaning is **inferred** from OpenRazer (0 = not stored, 1 = stored on device);
/// Synapse's own choice is unknown. See open questions in `vault/Status.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    Volatile = 0,
    Persistent = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dpi {
    pub x: u16,
    pub y: u16,
}

impl Dpi {
    pub fn uniform(dpi: u16) -> Self {
        Self { x: dpi, y: dpi }
    }
}

impl std::fmt::Display for Dpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.x == self.y {
            write!(f, "{}", self.x)
        } else {
            write!(f, "{}x{}", self.x, self.y)
        }
    }
}

pub fn get_dpi(storage: Storage) -> Request {
    fixed(GET_DPI, &[storage as u8])
}

/// `[storage, X u16, Y u16, Z u16 = 0]`.
pub fn set_dpi(storage: Storage, dpi: Dpi) -> Request {
    let [xh, xl] = dpi.x.to_be_bytes();
    let [yh, yl] = dpi.y.to_be_bytes();
    fixed(SET_DPI, &[storage as u8, xh, xl, yh, yl, 0, 0])
}

/// Reply: `[storage, X u16, Y u16, Z u16]`.
pub fn parse_dpi(args: &[u8]) -> Result<Dpi, ProtocolError> {
    need(args, 5)?;
    Ok(Dpi {
        x: be16(args, 1),
        y: be16(args, 3),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpiStage {
    pub id: u8,
    pub dpi: Dpi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpiStages {
    /// Stage ID (not index) of the active stage.
    pub active: u8,
    pub stages: Vec<DpiStage>,
}

const STAGE_LEN: usize = 7;
const STAGES_HEADER: usize = 3;
/// Most stages that fit in the 80-byte argument area.
pub const MAX_DPI_STAGES: usize = (crate::ARGS_LEN - STAGES_HEADER) / STAGE_LEN;

pub fn get_dpi_stages(storage: Storage) -> Request {
    fixed(GET_DPI_STAGES, &[storage as u8])
}

/// `[storage, active, n, n × (id, X u16, Y u16, Z u16)]`, data_size `7·n + 3`.
pub fn set_dpi_stages(storage: Storage, stages: &DpiStages) -> Result<Request, ProtocolError> {
    let n = stages.stages.len();
    if n == 0 || n > MAX_DPI_STAGES {
        return Err(ProtocolError::Range("number of DPI stages"));
    }
    let mut args = Vec::with_capacity(STAGES_HEADER + STAGE_LEN * n);
    args.extend_from_slice(&[storage as u8, stages.active, n as u8]);
    for s in &stages.stages {
        args.push(s.id);
        args.extend_from_slice(&s.dpi.x.to_be_bytes());
        args.extend_from_slice(&s.dpi.y.to_be_bytes());
        args.extend_from_slice(&[0, 0]);
    }
    Request::with_size(SET_DPI_STAGES, args.len() as u8, &args)
}

pub fn parse_dpi_stages(args: &[u8]) -> Result<DpiStages, ProtocolError> {
    need(args, STAGES_HEADER)?;
    let n = usize::from(args[2]);
    if n > MAX_DPI_STAGES {
        return Err(ProtocolError::Value {
            field: "DPI stage count",
            value: args[2],
        });
    }
    need(args, STAGES_HEADER + STAGE_LEN * n)?;
    let stages = args[STAGES_HEADER..STAGES_HEADER + STAGE_LEN * n]
        .as_chunks::<STAGE_LEN>()
        .0
        .iter()
        .map(|c| DpiStage {
            id: c[0],
            dpi: Dpi {
                x: be16(c, 1),
                y: be16(c, 3),
            },
        })
        .collect();
    Ok(DpiStages {
        active: args[1],
        stages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_dpi_bytes() {
        let r = set_dpi(Storage::Persistent, Dpi { x: 800, y: 1600 }).encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x04, 0x05, 7));
        assert_eq!(&r.args()[..8], &[1, 0x03, 0x20, 0x06, 0x40, 0, 0, 0]);
    }

    #[test]
    fn get_dpi_bytes() {
        let r = get_dpi(Storage::Volatile).encode(0);
        assert_eq!((r.command_id(), r.data_size()), (0x85, 7));
        assert_eq!(r.args()[0], 0);
    }

    #[test]
    fn parse_dpi_reply() {
        let d = parse_dpi(&[1, 0x75, 0x30, 0x00, 0x64, 0, 0]).unwrap();
        assert_eq!(d, Dpi { x: 30000, y: 100 });
        assert_eq!(d.to_string(), "30000x100");
        assert_eq!(Dpi::uniform(800).to_string(), "800");
    }

    fn sample() -> DpiStages {
        DpiStages {
            active: 2,
            stages: [400, 800, 1600]
                .iter()
                .enumerate()
                .map(|(i, &d)| DpiStage {
                    id: i as u8 + 1,
                    dpi: Dpi::uniform(d),
                })
                .collect(),
        }
    }

    #[test]
    fn stages_roundtrip() {
        let r = set_dpi_stages(Storage::Persistent, &sample())
            .unwrap()
            .encode(0);
        assert_eq!(r.data_size(), 7 * 3 + 3);
        assert_eq!(&r.args()[..10], &[1, 2, 3, 1, 0x01, 0x90, 0x01, 0x90, 0, 0]);
        assert_eq!(parse_dpi_stages(r.args()).unwrap(), sample());
    }

    #[test]
    fn stage_limits() {
        assert_eq!(MAX_DPI_STAGES, 11);
        let mut s = sample();
        s.stages.clear();
        assert!(set_dpi_stages(Storage::Volatile, &s).is_err());
        s.stages = vec![sample().stages[0]; 12];
        assert!(set_dpi_stages(Storage::Volatile, &s).is_err());
        let mut bad = [0u8; 80];
        bad[2] = 12;
        assert!(parse_dpi_stages(&bad).is_err());
    }
}
