//! Onboard button assignment (class 0x02). `vault/Protocol/Commands 2.5.md`.
//!
//! The data bytes per function type are **unknown**, so this module only frames
//! raw assignments. Typed functions come after a USB capture (phase 3).

use crate::command::*;
use crate::{ProtocolError, Request, fixed, need};

/// Mapping layer (observed: 0 normal, 1 HyperShift).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Normal = 0,
    HyperShift = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonAssignment {
    pub profile: u8,
    pub button: u8,
    pub layer: Layer,
    /// Function type (observed: 0 off, 1 mouse button, 2 key, 6 DPI, …).
    pub function: u8,
    pub data: Vec<u8>,
}

const HEADER: usize = 5;

pub fn get_button_assignment(profile: u8, button: u8, layer: Layer) -> Request {
    fixed(GET_BUTTON_ASSIGNMENT, &[profile, button, layer as u8])
}

/// `[profile, button, layer, function, dataLen, data…]`.
pub fn set_button_assignment(a: &ButtonAssignment) -> Result<Request, ProtocolError> {
    if a.data.len() > crate::ARGS_LEN - HEADER {
        return Err(ProtocolError::Range("button assignment data length"));
    }
    let mut args = vec![
        a.profile,
        a.button,
        a.layer as u8,
        a.function,
        a.data.len() as u8,
    ];
    args.extend_from_slice(&a.data);
    Request::new(SET_BUTTON_ASSIGNMENT, &args)
}

pub fn parse_button_assignment(args: &[u8]) -> Result<ButtonAssignment, ProtocolError> {
    need(args, HEADER)?;
    let layer = match args[2] {
        0 => Layer::Normal,
        1 => Layer::HyperShift,
        v => {
            return Err(ProtocolError::Value {
                field: "button layer",
                value: v,
            });
        }
    };
    let len = usize::from(args[4]);
    need(args, HEADER + len)?;
    Ok(ButtonAssignment {
        profile: args[0],
        button: args[1],
        layer,
        function: args[3],
        data: args[HEADER..HEADER + len].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let a = ButtonAssignment {
            profile: 1,
            button: 4,
            layer: Layer::HyperShift,
            function: 1,
            data: vec![1, 0],
        };
        let r = set_button_assignment(&a).unwrap().encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x02, 0x0C, 80));
        assert_eq!(&r.args()[..7], &[1, 4, 1, 1, 2, 1, 0]);
        assert_eq!(parse_button_assignment(r.args()).unwrap(), a);
    }

    #[test]
    fn get_request() {
        let r = get_button_assignment(1, 2, Layer::Normal).encode(0);
        assert_eq!(r.command_id(), 0x8C);
        assert_eq!(&r.args()[..3], &[1, 2, 0]);
    }

    #[test]
    fn bad_layer() {
        assert!(parse_button_assignment(&[1, 1, 7, 0, 0]).is_err());
    }
}
