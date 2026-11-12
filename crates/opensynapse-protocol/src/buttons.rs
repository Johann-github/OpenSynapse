//! Onboard button assignment (class 0x02). `vault/Protocol/Commands 2.5.md`.
//!
//! Data bytes are known for the function types in [`function`] (USB capture,
//! `vault/RE-Log/2026-11-09 Button remap capture.md`); others stay raw.

use crate::command::*;
use crate::{ProtocolError, Request, fixed, need};

/// Function type IDs whose data layout was seen in a capture (observed).
pub mod function {
    /// No data (`dataLen` 0): the button does nothing (observed, Synapse "Disable",
    /// `vault/RE-Log/2026-11-11 Button capture off and DPI down.md`).
    pub const OFF: u8 = 0;
    /// Data `[buttonId]`, e.g. `[4]` = mouse button 4.
    pub const MOUSE_BUTTON: u8 = 1;
    /// Data `[modifiers, usage]`; `[0x00, 0x04]` = key "A" (observed). Inferred:
    /// `modifiers` is the HID boot-keyboard modifier byte (0 = none) and `usage` the
    /// HID keyboard usage ID (see [`crate::hid_usage`]).
    pub const KEYBOARD: u8 = 2;
    /// Data `[action]`; `[1]` = DPI up, `[2]` = DPI down (both observed via Synapse,
    /// meaning inferred from the UI choice).
    pub const DPI: u8 = 6;
    /// Data `[buttonId]`; `[1]` = double left click (button inferred).
    pub const DOUBLE_CLICK: u8 = 11;
}

/// Profile byte for 02/0C · 02/8C that matches DPI `storage`: 0 = volatile (acts at
/// once, lost on power-off, flash untouched), 1 = onboard slot (persistent). Observed
/// on the Viper V3 HyperSpeed, `vault/RE-Log/2026-11-11 Button write storage.md`.
pub fn profile_for(storage: crate::dpi::Storage) -> u8 {
    storage as u8
}

/// Assign a plain mouse button (`1` left … `8`) to `button`.
pub fn mouse_button(profile: u8, button: u8, mode: Mode, target: u8) -> ButtonAssignment {
    ButtonAssignment {
        profile,
        button,
        mode,
        function: function::MOUSE_BUTTON,
        data: vec![target],
    }
}

/// Make `button` press keyboard key `usage` (HID usage ID) with `modifiers` held
/// (HID boot-keyboard modifier bits, 0 = none).
pub fn keyboard(profile: u8, button: u8, mode: Mode, modifiers: u8, usage: u8) -> ButtonAssignment {
    ButtonAssignment {
        profile,
        button,
        mode,
        function: function::KEYBOARD,
        data: vec![modifiers, usage],
    }
}

/// Make `button` step the DPI up.
pub fn dpi_up(profile: u8, button: u8, mode: Mode) -> ButtonAssignment {
    ButtonAssignment {
        profile,
        button,
        mode,
        function: function::DPI,
        data: vec![DPI_UP],
    }
}

/// Make `button` step the DPI down.
pub fn dpi_down(profile: u8, button: u8, mode: Mode) -> ButtonAssignment {
    ButtonAssignment {
        profile,
        button,
        mode,
        function: function::DPI,
        data: vec![DPI_DOWN],
    }
}

/// Make `button` do nothing.
pub fn off(profile: u8, button: u8, mode: Mode) -> ButtonAssignment {
    ButtonAssignment {
        profile,
        button,
        mode,
        function: function::OFF,
        data: vec![],
    }
}

/// Data byte of [`function::DPI`] for "DPI up" (observed).
const DPI_UP: u8 = 1;
/// Data byte of [`function::DPI`] for "DPI down" (observed).
const DPI_DOWN: u8 = 2;

/// Make `button` send a double click of `target` (1 = left).
pub fn double_click(profile: u8, button: u8, mode: Mode, target: u8) -> ButtonAssignment {
    ButtonAssignment {
        profile,
        button,
        mode,
        function: function::DOUBLE_CLICK,
        data: vec![target],
    }
}

/// Byte 2 of 02/0C (set) and 02/8C (get): the mapping mode.
///
/// Evidence: Synapse 4 names this byte "mode" with `NormalMode = 0` and
/// `HypershiftMode = 1` (observed in code: `index-42d42381.js`, rzDevice25 enum,
/// `getSingleButtonAssignment` / `setSingleButtonAssignment`). The Viper V3
/// HyperSpeed keeps a separate HyperShift slot per button: a mode 1 write changed
/// only the mode 1 read (02/8C and 02/8E), the mode 0 mapping stayed (observed on
/// hardware, 2026-11-10, see [`parse_button_assignment`]). Factory defaults are the
/// same in both modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Normal = 0,
    HyperShift = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonAssignment {
    pub profile: u8,
    pub button: u8,
    pub mode: Mode,
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
    /// Keyboard key: HID modifier bits and usage ID.
    Key { modifiers: u8, usage: u8 },
    /// Step DPI up.
    DpiUp,
    /// Step DPI down.
    DpiDown,
    /// Button disabled.
    Off,
}

impl ButtonAssignment {
    /// The decoded action, if function and data match a known layout.
    pub fn known(&self) -> Option<Known> {
        match (self.function, self.data.as_slice()) {
            (function::MOUSE_BUTTON, &[t]) => Some(Known::MouseButton(t)),
            (function::DOUBLE_CLICK, &[t]) => Some(Known::DoubleClick(t)),
            (function::KEYBOARD, &[modifiers, usage]) => Some(Known::Key { modifiers, usage }),
            (function::DPI, &[DPI_UP]) => Some(Known::DpiUp),
            (function::DPI, &[DPI_DOWN]) => Some(Known::DpiDown),
            (function::OFF, &[]) => Some(Known::Off),
            _ => None,
        }
    }
}

impl std::fmt::Display for Known {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Known::MouseButton(t) => write!(f, "mouse button {t}"),
            Known::DoubleClick(t) => write!(f, "double-click {t}"),
            Known::Key { modifiers, usage } => {
                write!(f, "key {usage:#04x} (mods {modifiers:#04x})")
            }
            Known::DpiUp => f.write_str("DPI up"),
            Known::DpiDown => f.write_str("DPI down"),
            Known::Off => f.write_str("off"),
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Mode::Normal => "normal",
            Mode::HyperShift => "HyperShift",
        })
    }
}

/// `profile 1, button 4, normal mode: mouse button 4 (function 1, data [4])`.
impl std::fmt::Display for ButtonAssignment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "profile {}, button {}, {} mode: ",
            self.profile, self.button, self.mode
        )?;
        if let Some(k) = self.known() {
            write!(f, "{k} ")?;
        }
        write!(f, "(function {}, data {:?})", self.function, self.data)
    }
}

/// A parsed 02/8C reply: the assignment (with the *requested* mode) plus the raw
/// reply byte 2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonReply {
    pub assignment: ButtonAssignment,
    /// Reply byte 2 as sent by the device; meaning unknown. It is not the mode of
    /// the slot that was read (see [`parse_button_assignment`]).
    pub reply_byte2: u8,
}

/// Same as the assignment; [`ButtonReply::reply_byte2`] is not shown.
impl std::fmt::Display for ButtonReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.assignment, f)
    }
}

const HEADER: usize = 5;

/// `[profile, button, mode]`, data_size 80 (observed in code: Synapse 4
/// `getSingleButtonAssignment`).
pub fn get_button_assignment(profile: u8, button: u8, mode: Mode) -> Request {
    fixed(GET_BUTTON_ASSIGNMENT, &[profile, button, mode as u8])
}

/// `[profile, button, mode, function, dataLen, data…]`.
pub fn set_button_assignment(a: &ButtonAssignment) -> Result<Request, ProtocolError> {
    if a.data.len() > crate::ARGS_LEN - HEADER {
        return Err(ProtocolError::Range("button assignment data length"));
    }
    let mut args = vec![
        a.profile,
        a.button,
        a.mode as u8,
        a.function,
        a.data.len() as u8,
    ];
    args.extend_from_slice(&a.data);
    // Observed size 10; only longer data (macros etc.) needs more.
    let size = SET_BUTTON_ASSIGNMENT.size.max(args.len() as u8);
    Request::with_size(SET_BUTTON_ASSIGNMENT, size, &args)
}

/// Parse a 02/8C reply `[profile, button, byte2, function, dataLen, data…]` for a
/// request made with `requested`.
///
/// The device answers from the requested mode's slot (observed on hardware,
/// 2026-11-10, Viper V3 HyperSpeed): after button 5 was set to mouse button 4 in
/// mode 1, request `01 05 01` replied `01 05 01 01 01 04` while `01 05 00` still
/// replied `01 05 01 01 01 05`. So the result's `mode` is `requested`.
///
/// Reply byte 2 is not that mode: for button 5 it was 1 for both requests, and for
/// button 4 (last written with mode 0) it was 0 for both (observed). Synapse 4 reads
/// it as the mode (observed in code: reply parser in `index-42d42381.js`). Its
/// meaning is unknown (inferred guess: the mode of the most recent write to that
/// button; factory value 1). It is kept raw in [`ButtonReply::reply_byte2`] (any
/// value accepted).
pub fn parse_button_assignment(args: &[u8], requested: Mode) -> Result<ButtonReply, ProtocolError> {
    need(args, HEADER)?;
    let len = usize::from(args[4]);
    need(args, HEADER + len)?;
    Ok(ButtonReply {
        assignment: ButtonAssignment {
            profile: args[0],
            button: args[1],
            mode: requested,
            function: args[3],
            data: args[HEADER..HEADER + len].to_vec(),
        },
        reply_byte2: args[2],
    })
}

/// Header of 02/8E: `[profile, mode, idOffset]`.
const LIST_HEADER: usize = 3;

/// 02/8E "Get Button Assignment List": `[profile, mode, idOffset]`, data_size 80
/// (observed in code: Synapse 4 rzDevice25). The reply honours the mode (observed
/// on hardware, 2026-11-10).
pub fn get_button_assignment_list(profile: u8, mode: Mode, id_offset: u8) -> Request {
    fixed(
        GET_BUTTON_ASSIGNMENT_LIST,
        &[profile, mode as u8, id_offset],
    )
}

/// Size of one 02/8E record: `[buttonId, function, dataLen, data×5]` (inferred).
const LIST_RECORD: usize = 8;

/// A 02/8E reply `[profile, mode, idOffset, data…]`; `mode` is the raw byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonAssignmentList {
    pub profile: u8,
    pub mode: u8,
    pub id_offset: u8,
    /// Everything after the header (the rest of the 80-byte argument area).
    pub data: Vec<u8>,
}

impl ButtonAssignmentList {
    /// The records in [`Self::data`] as `(button_id, function, data)`.
    ///
    /// Layout inferred from Viper V3 HyperSpeed replies (2026-11-10): 8-byte records
    /// `[buttonId, function, dataLen, data×5]`, where only `dataLen` data bytes count;
    /// the list ends at a zero button ID or when no full record is left. A `dataLen`
    /// above 5 is cut to 5.
    pub fn records(&self) -> Vec<(u8, u8, Vec<u8>)> {
        self.data
            .as_chunks::<LIST_RECORD>()
            .0
            .iter()
            .take_while(|r| r[0] != 0)
            .map(|r| {
                let len = usize::from(r[2]).min(LIST_RECORD - 3);
                (r[0], r[1], r[3..3 + len].to_vec())
            })
            .collect()
    }
}

pub fn parse_button_assignment_list(args: &[u8]) -> Result<ButtonAssignmentList, ProtocolError> {
    need(args, LIST_HEADER)?;
    Ok(ButtonAssignmentList {
        profile: args[0],
        mode: args[1],
        id_offset: args[2],
        data: args[LIST_HEADER..].to_vec(),
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
            mode: Mode::HyperShift,
            function: 1,
            data: vec![1, 0],
        };
        let r = set_button_assignment(&a).unwrap().encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x02, 0x0C, 10));
        assert_eq!(&r.args()[..7], &[1, 4, 1, 1, 2, 1, 0]);
        let back = parse_button_assignment(r.args(), Mode::HyperShift).unwrap();
        assert_eq!(back.assignment, a);
        assert_eq!(back.reply_byte2, 1);
    }

    /// Byte-exact against the Synapse capture (2026-11-09, Viper V3 HyperSpeed).
    #[test]
    fn matches_capture() {
        let dbl = set_button_assignment(&double_click(1, 4, Mode::Normal, 1))
            .unwrap()
            .encode(0x06);
        let mut want = [0u8; 90];
        want[..14].copy_from_slice(&[0x00, 0x06, 0, 0, 0, 0x0a, 0x02, 0x0c, 1, 4, 0, 0x0b, 1, 1]);
        want[88] = 0x0a;
        assert_eq!(dbl.as_bytes(), &want);

        let back = set_button_assignment(&mouse_button(1, 4, Mode::Normal, 4))
            .unwrap()
            .encode(0x07);
        assert_eq!(
            &back.as_bytes()[..14],
            &[0x00, 0x07, 0, 0, 0, 0x0a, 0x02, 0x0c, 1, 4, 0, 0x01, 1, 4,]
        );
        assert_eq!(back.as_bytes()[88], 0x05);
    }

    /// Expected CRC: XOR of bytes 2..=87.
    fn xor_crc(b: &[u8]) -> u8 {
        b[2..88].iter().fold(0, |c, x| c ^ x)
    }

    /// Byte-exact against the Synapse capture (button 4 -> key "A", -> DPI up).
    #[test]
    fn matches_capture_key_and_dpi() {
        let key = set_button_assignment(&keyboard(1, 4, Mode::Normal, 0x00, 0x04))
            .unwrap()
            .encode(0x08);
        let mut want = [0u8; 90];
        want[..15].copy_from_slice(&[
            0x00, 0x08, 0, 0, 0, 0x0a, 0x02, 0x0c, 0x01, 0x04, 0x00, 0x02, 0x02, 0x00, 0x04,
        ]);
        want[88] = xor_crc(&want);
        assert_eq!(key.as_bytes(), &want);

        let dpi = set_button_assignment(&dpi_up(1, 4, Mode::Normal))
            .unwrap()
            .encode(0x09);
        let mut want = [0u8; 90];
        want[..14].copy_from_slice(&[
            0x00, 0x09, 0, 0, 0, 0x0a, 0x02, 0x0c, 0x01, 0x04, 0x00, 0x06, 0x01, 0x01,
        ]);
        want[88] = xor_crc(&want);
        assert_eq!(dpi.as_bytes(), &want);
    }

    /// `vault/RE-Log/2026-11-11 Button capture off and DPI down.md`.
    #[test]
    fn matches_capture_off_and_dpi_down() {
        let frame = |a: &ButtonAssignment, tid: u8| set_button_assignment(a).unwrap().encode(tid);
        let mut want = [0u8; 90];
        want[..13].copy_from_slice(&[
            0x00, 0x01, 0, 0, 0, 0x0a, 0x02, 0x0c, 0x01, 0x04, 0x00, 0x00, 0x00,
        ]);
        want[88] = xor_crc(&want);
        assert_eq!(frame(&off(1, 4, Mode::Normal), 0x01).as_bytes(), &want);

        let mut want = [0u8; 90];
        want[..14].copy_from_slice(&[
            0x00, 0x02, 0, 0, 0, 0x0a, 0x02, 0x0c, 0x01, 0x04, 0x00, 0x06, 0x01, 0x02,
        ]);
        want[88] = xor_crc(&want);
        assert_eq!(frame(&dpi_down(1, 4, Mode::Normal), 0x02).as_bytes(), &want);
    }

    #[test]
    fn long_data_grows_size() {
        let mut a = mouse_button(1, 4, Mode::Normal, 4);
        a.data = vec![0; 20];
        let r = set_button_assignment(&a).unwrap().encode(0);
        assert_eq!(r.data_size(), 25);
    }

    #[test]
    fn get_request() {
        let r = get_button_assignment(1, 2, Mode::Normal).encode(0);
        assert_eq!((r.command_id(), r.data_size()), (0x8C, 80));
        assert_eq!(&r.args()[..3], &[1, 2, 0]);
        let r = get_button_assignment(1, 2, Mode::HyperShift).encode(0);
        assert_eq!(&r.args()[..3], &[1, 2, 1]);
    }

    /// 80-byte argument area starting with `head`.
    fn reply(head: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; crate::ARGS_LEN];
        v[..head.len()].copy_from_slice(head);
        v
    }

    /// Viper V3 HyperSpeed replies (hardware, 2026-11-10) for buttons whose two slots
    /// hold the same mapping: the result carries the requested mode, and reply byte 2
    /// is kept raw whatever the request was.
    #[test]
    fn requested_mode_wins_and_byte2_is_raw() {
        // Button 1, factory default: byte 2 is 1 for both requests.
        let r = reply(&[0x01, 0x01, 0x01, 0x01, 0x01, 0x01]);
        let n = parse_button_assignment(&r, Mode::Normal).unwrap();
        assert_eq!(n.assignment, mouse_button(1, 1, Mode::Normal, 1));
        assert_eq!(n.reply_byte2, 1);
        assert_eq!(
            n.to_string(),
            "profile 1, button 1, normal mode: mouse button 1 (function 1, data [1])"
        );
        let h = parse_button_assignment(&r, Mode::HyperShift).unwrap();
        assert_eq!(h.assignment, mouse_button(1, 1, Mode::HyperShift, 1));

        // Button 4, last written with mode 0: byte 2 is 0 for both requests.
        let r = reply(&[0x01, 0x04, 0x00, 0x01, 0x01, 0x04]);
        let n = parse_button_assignment(&r, Mode::Normal).unwrap();
        assert_eq!(n.assignment, mouse_button(1, 4, Mode::Normal, 4));
        let h = parse_button_assignment(&r, Mode::HyperShift).unwrap();
        assert_eq!(h.assignment, mouse_button(1, 4, Mode::HyperShift, 4));
        assert_eq!(h.reply_byte2, 0);
        assert!(
            h.to_string()
                .ends_with("HyperShift mode: mouse button 4 (function 1, data [4])")
        );
        // Re-encoding a read uses the requested mode, not byte 2.
        let req = set_button_assignment(&h.assignment).unwrap().encode(0);
        assert_eq!(&req.args()[..3], &[1, 4, 1]);
    }

    /// Hardware, 2026-11-10: after `buttons set 5 mouse-button 4 --hypershift`, the
    /// two slots of button 5 differ; byte 2 is 1 in both replies.
    #[test]
    fn hypershift_slot_is_separate() {
        let h = parse_button_assignment(
            &reply(&[0x01, 0x05, 0x01, 0x01, 0x01, 0x04]),
            Mode::HyperShift,
        )
        .unwrap();
        assert_eq!(h.assignment, mouse_button(1, 5, Mode::HyperShift, 4));
        assert_eq!(h.reply_byte2, 1);
        let n =
            parse_button_assignment(&reply(&[0x01, 0x05, 0x01, 0x01, 0x01, 0x05]), Mode::Normal)
                .unwrap();
        assert_eq!(n.assignment, mouse_button(1, 5, Mode::Normal, 5));
        assert_eq!(n.reply_byte2, 1);
    }

    #[test]
    fn any_byte2_value_is_accepted() {
        let r = parse_button_assignment(&[1, 1, 7, 0, 0], Mode::Normal).unwrap();
        assert_eq!((r.assignment.mode, r.reply_byte2), (Mode::Normal, 7));
        assert!(!r.to_string().contains('7'));
    }

    #[test]
    fn short_reply() {
        assert!(parse_button_assignment(&[1, 1, 0, 1], Mode::Normal).is_err());
        assert!(parse_button_assignment(&[1, 1, 0, 1, 2, 4], Mode::Normal).is_err());
    }

    #[test]
    fn known_and_display() {
        let a = mouse_button(1, 4, Mode::Normal, 5);
        assert_eq!(a.known(), Some(Known::MouseButton(5)));
        assert_eq!(
            a.to_string(),
            "profile 1, button 4, normal mode: mouse button 5 (function 1, data [5])"
        );
        let d = double_click(2, 5, Mode::HyperShift, 1);
        assert_eq!(d.known(), Some(Known::DoubleClick(1)));
        assert!(d.to_string().contains("HyperShift mode: double-click 1"));
        let k = keyboard(1, 4, Mode::Normal, 0x00, 0x04);
        assert_eq!(
            k.known(),
            Some(Known::Key {
                modifiers: 0,
                usage: 4
            })
        );
        assert!(
            k.to_string()
                .ends_with("mode: key 0x04 (mods 0x00) (function 2, data [0, 4])")
        );
        let up = dpi_up(1, 4, Mode::Normal);
        assert_eq!(up.known(), Some(Known::DpiUp));
        assert!(
            up.to_string()
                .contains("mode: DPI up (function 6, data [1])")
        );
        let down = dpi_down(1, 4, Mode::Normal);
        assert_eq!(down.known(), Some(Known::DpiDown));
        assert_eq!(off(1, 4, Mode::Normal).known(), Some(Known::Off));
        // DPI data other than up/down is not observed and stays raw.
        let raw = ButtonAssignment {
            data: vec![3],
            ..up
        };
        assert_eq!(raw.known(), None);
        assert!(raw.to_string().ends_with("mode: (function 6, data [3])"));
        // Known function with unexpected data length stays raw.
        let odd = ButtonAssignment {
            data: vec![1, 0],
            ..a
        };
        assert_eq!(odd.known(), None);
    }

    #[test]
    fn list_request_and_raw_reply() {
        let r = get_button_assignment_list(1, Mode::HyperShift, 8).encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x02, 0x8E, 80));
        assert_eq!(&r.args()[..4], &[1, 1, 8, 0]);

        let l = parse_button_assignment_list(&reply(&[1, 0, 0, 0xAA, 0xBB])).unwrap();
        assert_eq!((l.profile, l.mode, l.id_offset), (1, 0, 0));
        assert_eq!(l.data.len(), crate::ARGS_LEN - 3);
        assert_eq!(&l.data[..2], &[0xAA, 0xBB]);
        assert!(parse_button_assignment_list(&[1, 0]).is_err());
    }

    /// Viper V3 HyperSpeed 02/8E reply data, profile 1, mode 0 (hardware, 2026-11-10).
    #[test]
    fn list_records_from_observed_reply() {
        #[rustfmt::skip]
        let data = [
            0x01, 0x01, 0x01, 0x01, 0, 0, 0, 0,
            0x02, 0x01, 0x01, 0x02, 0, 0, 0, 0,
            0x03, 0x01, 0x01, 0x03, 0, 0, 0, 0,
            0x04, 0x01, 0x01, 0x04, 0, 0, 0, 0,
            0x05, 0x01, 0x01, 0x05, 0, 0, 0, 0,
            0x60, 0x06, 0x01, 0x06, 0, 0, 0, 0,
            0x09, 0x01, 0x01, 0x09, 0, 0, 0, 0,
            0x0a, 0x01, 0x01, 0x0a,
        ];
        let mut args = vec![1, 0, 0];
        args.extend_from_slice(&data);
        let l = parse_button_assignment_list(&reply(&args)).unwrap();
        let recs = l.records();
        assert_eq!(
            recs,
            [
                (0x01, 1, vec![1]),
                (0x02, 1, vec![2]),
                (0x03, 1, vec![3]),
                (0x04, 1, vec![4]),
                (0x05, 1, vec![5]),
                (0x60, 6, vec![6]),
                (0x09, 1, vec![9]),
                (0x0a, 1, vec![10]),
            ]
        );
        // Mode 1 after the HyperShift write: button 5 -> mouse button 4.
        let mut args = vec![1, 1, 0, 0x05, 0x01, 0x01, 0x04];
        args.resize(crate::ARGS_LEN, 0);
        let l = parse_button_assignment_list(&args).unwrap();
        assert_eq!(l.records(), [(5, 1, vec![4])]);
        // Over-long dataLen is cut to the record; a partial record is ignored.
        let l = ButtonAssignmentList {
            profile: 1,
            mode: 0,
            id_offset: 0,
            data: vec![7, 2, 9, 1, 2, 3, 4, 5, 8, 1, 1],
        };
        assert_eq!(l.records(), [(7, 2, vec![1, 2, 3, 4, 5])]);
    }
}
