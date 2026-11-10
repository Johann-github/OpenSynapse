//! Onboard button assignment (class 0x02). `vault/Protocol/Commands 2.5.md`.
//!
//! Data bytes are known for the function types in [`function`] (USB capture,
//! `vault/RE-Log/2026-11-09 Button remap capture.md`); others stay raw.

use crate::command::*;
use crate::{ProtocolError, Request, fixed, need};

/// Function type IDs whose data layout was seen in a capture (observed).
pub mod function {
    /// Data `[buttonId]`, e.g. `[4]` = mouse button 4.
    pub const MOUSE_BUTTON: u8 = 1;
    /// Data `[buttonId]`; `[1]` = double left click (button inferred).
    pub const DOUBLE_CLICK: u8 = 11;
}

/// Assign a plain mouse button (`1` left … `8`) to `button`.
pub fn mouse_button(profile: u8, button: u8, layer: Layer, target: u8) -> ButtonAssignment {
    ButtonAssignment {
        profile,
        button,
        layer,
        function: function::MOUSE_BUTTON,
        data: vec![target],
    }
}

/// Make `button` send a double click of `target` (1 = left).
pub fn double_click(profile: u8, button: u8, layer: Layer, target: u8) -> ButtonAssignment {
    ButtonAssignment {
        profile,
        button,
        layer,
        function: function::DOUBLE_CLICK,
        data: vec![target],
    }
}

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

/// An assignment whose data layout is known (see [`function`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Known {
    /// Plain mouse button (`1` left …).
    MouseButton(u8),
    /// Double click of the given button (`1` left).
    DoubleClick(u8),
}

impl ButtonAssignment {
    /// The decoded action, if function and data match a known layout.
    pub fn known(&self) -> Option<Known> {
        match (self.function, self.data.as_slice()) {
            (function::MOUSE_BUTTON, &[t]) => Some(Known::MouseButton(t)),
            (function::DOUBLE_CLICK, &[t]) => Some(Known::DoubleClick(t)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Known {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Known::MouseButton(t) => write!(f, "mouse button {t}"),
            Known::DoubleClick(t) => write!(f, "double-click {t}"),
        }
    }
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Layer::Normal => "normal",
            Layer::HyperShift => "hypershift",
        })
    }
}

/// `profile 1, button 4, normal layer: mouse button 4 (function 1, data [4])`.
impl std::fmt::Display for ButtonAssignment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "profile {}, button {}, {} layer: ",
            self.profile, self.button, self.layer
        )?;
        if let Some(k) = self.known() {
            write!(f, "{k} ")?;
        }
        write!(f, "(function {}, data {:?})", self.function, self.data)
    }
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
    // Observed size 10; only longer data (macros etc.) needs more.
    let size = SET_BUTTON_ASSIGNMENT.size.max(args.len() as u8);
    Request::with_size(SET_BUTTON_ASSIGNMENT, size, &args)
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
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x02, 0x0C, 10));
        assert_eq!(&r.args()[..7], &[1, 4, 1, 1, 2, 1, 0]);
        assert_eq!(parse_button_assignment(r.args()).unwrap(), a);
    }

    /// Byte-exact against the Synapse capture (2026-11-09, Viper V3 HyperSpeed).
    #[test]
    fn matches_capture() {
        let dbl = set_button_assignment(&double_click(1, 4, Layer::Normal, 1))
            .unwrap()
            .encode(0x06);
        let mut want = [0u8; 90];
        want[..14].copy_from_slice(&[0x00, 0x06, 0, 0, 0, 0x0a, 0x02, 0x0c, 1, 4, 0, 0x0b, 1, 1]);
        want[88] = 0x0a;
        assert_eq!(dbl.as_bytes(), &want);

        let back = set_button_assignment(&mouse_button(1, 4, Layer::Normal, 4))
            .unwrap()
            .encode(0x07);
        assert_eq!(
            &back.as_bytes()[..14],
            &[0x00, 0x07, 0, 0, 0, 0x0a, 0x02, 0x0c, 1, 4, 0, 0x01, 1, 4,]
        );
        assert_eq!(back.as_bytes()[88], 0x05);
    }

    #[test]
    fn long_data_grows_size() {
        let mut a = mouse_button(1, 4, Layer::Normal, 4);
        a.data = vec![0; 20];
        let r = set_button_assignment(&a).unwrap().encode(0);
        assert_eq!(r.data_size(), 25);
    }

    #[test]
    fn get_request() {
        let r = get_button_assignment(1, 2, Layer::Normal).encode(0);
        assert_eq!(r.command_id(), 0x8C);
        assert_eq!(&r.args()[..3], &[1, 2, 0]);
    }

    #[test]
    fn known_and_display() {
        let a = mouse_button(1, 4, Layer::Normal, 5);
        assert_eq!(a.known(), Some(Known::MouseButton(5)));
        assert_eq!(
            a.to_string(),
            "profile 1, button 4, normal layer: mouse button 5 (function 1, data [5])"
        );
        let d = double_click(2, 5, Layer::HyperShift, 1);
        assert_eq!(d.known(), Some(Known::DoubleClick(1)));
        assert!(d.to_string().contains("hypershift layer: double-click 1"));
        let raw = ButtonAssignment {
            data: vec![0, 4],
            function: 2,
            ..a.clone()
        };
        assert_eq!(raw.known(), None);
        assert!(
            raw.to_string()
                .ends_with("layer: (function 2, data [0, 4])")
        );
        // Known function with unexpected data length stays raw.
        let odd = ButtonAssignment {
            data: vec![1, 0],
            ..a
        };
        assert_eq!(odd.known(), None);
    }

    #[test]
    fn bad_layer() {
        assert!(parse_button_assignment(&[1, 1, 7, 0, 0]).is_err());
    }
}
