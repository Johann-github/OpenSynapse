//! Lighting: extended (class 0x0F), standard matrix (03/0A) and legacy LED (class 0x03).
//!
//! Implements `vault/Protocol/Lighting 2.5.md`. Nothing here is hardware-tested (the
//! Viper V3 HyperSpeed has no lighting); layouts are observed in Synapse 4 code unless a
//! comment says inferred.

use std::fmt;
use std::str::FromStr;

use crate::command::*;
use crate::{ProtocolError, Request, fixed, need};

/// Storage byte of the extended commands: 0 volatile, 1 saved (meaning inferred).
pub use crate::dpi::Storage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl fmt::Display for Rgb {
    /// `00ff00`, the form [`FromStr`] accepts.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

impl FromStr for Rgb {
    type Err = ProtocolError;

    /// `00ff00`, `#00FF00` or `0x00ff00`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let t = t
            .strip_prefix('#')
            .or_else(|| t.strip_prefix("0x"))
            .unwrap_or(t);
        if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ProtocolError::Range(
                "colour (expected 6 hex digits, e.g. 00ff00)",
            ));
        }
        let byte = |i: usize| {
            u8::from_str_radix(&t[i..i + 2], 16)
                .map_err(|_| ProtocolError::Range("colour (expected 6 hex digits, e.g. 00ff00)"))
        };
        Ok(Rgb(byte(0)?, byte(2)?, byte(4)?))
    }
}

/// Effect IDs for class 0x0F (observed).
pub mod effect {
    pub const OFF: u8 = 0;
    pub const STATIC: u8 = 1;
    pub const BREATHING: u8 = 2;
    pub const SPECTRUM: u8 = 3;
    pub const WAVE: u8 = 4;
    pub const REACTIVE: u8 = 5;
}

/// Zone (region / LED) IDs, shared by class 0x0F and class 0x03 (observed).
pub mod region {
    pub const ALL: u8 = 0;
    pub const WHEEL: u8 = 1;
    pub const DPI_LED: u8 = 2;
    pub const BATTERY: u8 = 3;
    pub const LOGO: u8 = 4;
    pub const BACKLIGHT: u8 = 5;
    pub const UNDERGLOW: u8 = 10;
    pub const SIDE_STRIPE: u8 = 11;
    pub const RIGHT_SIDE: u8 = 16;
    pub const LEFT_SIDE: u8 = 17;

    const NAMES: [(u8, &str); 10] = [
        (ALL, "all"),
        (WHEEL, "wheel"),
        (DPI_LED, "dpi"),
        (BATTERY, "battery"),
        (LOGO, "logo"),
        (BACKLIGHT, "backlight"),
        (UNDERGLOW, "underglow"),
        (SIDE_STRIPE, "side-stripe"),
        (RIGHT_SIDE, "right-side"),
        (LEFT_SIDE, "left-side"),
    ];

    /// CLI/GUI name of a zone ID, or `zone N` for IDs without a name here.
    pub fn name(id: u8) -> String {
        NAMES
            .iter()
            .find(|(i, _)| *i == id)
            .map_or_else(|| format!("zone {id}"), |(_, n)| (*n).to_string())
    }

    /// Zone ID from a name above or a number.
    pub fn parse(s: &str) -> Option<u8> {
        let s = s.trim().to_ascii_lowercase();
        NAMES
            .iter()
            .find(|(_, n)| *n == s)
            .map(|(i, _)| *i)
            .or_else(|| s.parse().ok())
    }
}

/// Wave direction byte (values inferred from OpenRazer; which one is "left" is unknown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WaveDirection {
    One = 1,
    Two = 2,
}

impl WaveDirection {
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            _ => None,
        }
    }
}

/// The hardware effects OpenSynapse writes ([[Decisions/ADR-0014 Lighting]]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Off,
    Static(Rgb),
    Breathing(Rgb),
    BreathingDual(Rgb, Rgb),
    Spectrum,
    Wave(WaveDirection),
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => f.write_str("off"),
            Self::Static(c) => write!(f, "static {c}"),
            Self::Breathing(c) => write!(f, "breathing {c}"),
            Self::BreathingDual(a, b) => write!(f, "breathing {a} {b}"),
            Self::Spectrum => f.write_str("spectrum"),
            Self::Wave(d) => write!(f, "wave {}", *d as u8),
        }
    }
}

/// Wave speed byte sent with extended waves (inferred, OpenRazer default).
pub const WAVE_RATE: u8 = 0x28;

/// Raw class-0x0F effect: `[storage, zone, effect, flags, rate, n, n × RGB]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    /// Byte 0; Synapse calls it "profile" ([`Storage`] values, meaning inferred).
    pub profile: u8,
    pub region: u8,
    pub effect: u8,
    pub flags: u8,
    pub rate: u8,
    pub colors: Vec<Rgb>,
}

impl Effect {
    /// The raw effect for `mode` (per-effect flags/rate: `vault/Protocol/Lighting 2.5.md`).
    pub fn from_mode(storage: Storage, zone: u8, mode: Mode) -> Self {
        let (effect, flags, rate, colors) = match mode {
            Mode::Off => (effect::OFF, 0, 0, vec![]),
            Mode::Static(c) => (effect::STATIC, 0, 0, vec![c]),
            Mode::Breathing(c) => (effect::BREATHING, 1, 0, vec![c]),
            Mode::BreathingDual(a, b) => (effect::BREATHING, 2, 0, vec![a, b]),
            Mode::Spectrum => (effect::SPECTRUM, 0, 0, vec![]),
            Mode::Wave(d) => (effect::WAVE, d as u8, WAVE_RATE, vec![]),
        };
        Self {
            profile: storage as u8,
            region: zone,
            effect,
            flags,
            rate,
            colors,
        }
    }

    /// The [`Mode`] this effect is, or `None` for effects OpenSynapse does not write
    /// (reactive, custom frame, …). Flags/rate are matched loosely for read-back:
    /// breathing is told apart by its colour count, wave only needs a known direction.
    pub fn mode(&self) -> Option<Mode> {
        let c = &self.colors;
        match (self.effect, c.len()) {
            (effect::OFF, _) => Some(Mode::Off),
            (effect::STATIC, 1..) => Some(Mode::Static(c[0])),
            (effect::BREATHING, 1) => Some(Mode::Breathing(c[0])),
            (effect::BREATHING, 2) => Some(Mode::BreathingDual(c[0], c[1])),
            (effect::SPECTRUM, _) => Some(Mode::Spectrum),
            (effect::WAVE, _) => WaveDirection::from_byte(self.flags).map(Mode::Wave),
            _ => None,
        }
    }
}

impl Effect {
    /// The form OpenSynapse writes for the same [`Mode`] (keeping byte 0 and the
    /// zone), so a read-back compares equal to the request even when the device
    /// reports other flag/rate bytes. Effects without a mode stay raw.
    pub fn canonical(&self) -> Self {
        match self.mode() {
            Some(m) => Self {
                profile: self.profile,
                ..Self::from_mode(Storage::Volatile, self.region, m)
            },
            None => self.clone(),
        }
    }
}

const EFFECT_HEADER: usize = 6;

/// 0F/02: `[profile, region, effect, flags, rate, n, n × RGB]`, data_size `6 + 3·n`.
pub fn set_effect(e: &Effect) -> Result<Request, ProtocolError> {
    if EFFECT_HEADER + 3 * e.colors.len() > crate::ARGS_LEN {
        return Err(ProtocolError::Range("number of effect colors"));
    }
    let mut args = vec![
        e.profile,
        e.region,
        e.effect,
        e.flags,
        e.rate,
        e.colors.len() as u8,
    ];
    for c in &e.colors {
        args.extend_from_slice(&[c.0, c.1, c.2]);
    }
    Request::with_size(SET_EFFECT, args.len() as u8, &args)
}

/// 0F/02 for one of the [`Mode`]s.
pub fn set_mode(storage: Storage, zone: u8, mode: Mode) -> Request {
    set_effect(&Effect::from_mode(storage, zone, mode)).expect("at most two colours fit")
}

/// 0F/82.
pub fn get_effect(profile: u8, region: u8) -> Request {
    fixed(GET_EFFECT, &[profile, region])
}

pub fn parse_effect(args: &[u8]) -> Result<Effect, ProtocolError> {
    need(args, EFFECT_HEADER)?;
    let n = usize::from(args[5]);
    need(args, EFFECT_HEADER + 3 * n)?;
    Ok(Effect {
        profile: args[0],
        region: args[1],
        effect: args[2],
        flags: args[3],
        rate: args[4],
        colors: args[EFFECT_HEADER..EFFECT_HEADER + 3 * n]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[r, g, b]| Rgb(r, g, b))
            .collect(),
    })
}

/// 0F/84.
pub fn get_brightness(profile: u8, region: u8) -> Request {
    fixed(GET_BRIGHTNESS, &[profile, region])
}

/// 0F/04 with a raw value 0–255.
pub fn set_brightness(profile: u8, region: u8, value: u8) -> Request {
    fixed(SET_BRIGHTNESS, &[profile, region, value])
}

/// Reply `[profile, region, value]`.
pub fn parse_brightness(args: &[u8]) -> Result<u8, ProtocolError> {
    need(args, 3)?;
    Ok(args[2])
}

/// Percent (clamped to 100) → raw value as Synapse sends it to class 0x0F (floor).
pub fn percent_to_raw(percent: u8) -> u8 {
    (u16::from(percent.min(100)) * 255 / 100) as u8
}

/// Raw class-0x0F value → percent as Synapse shows it (ceil). Inverse of
/// [`percent_to_raw`] for every percent 0–100.
pub fn raw_to_percent(raw: u8) -> u8 {
    (u16::from(raw) * 100).div_ceil(255) as u8
}

/// Standard matrix family: 03/0A effect (whole device, no storage byte, no GET) and
/// LED brightness 03/03 · 03/83.
pub mod standard {
    use super::*;

    /// Effect enum of 03/0A (observed; differs from the 0x0F IDs).
    pub mod effect {
        pub const OFF: u8 = 0;
        pub const WAVE: u8 = 1;
        pub const REACTIVE: u8 = 2;
        pub const BREATHING: u8 = 3;
        pub const SPECTRUM: u8 = 4;
        pub const CUSTOM_FRAME: u8 = 5;
        pub const STATIC: u8 = 6;
    }

    /// ledClass byte OpenSynapse sends with LED brightness (inferred: "var store").
    pub const LED_CLASS: u8 = 1;

    /// 03/0A with the per-effect data_size Synapse uses.
    pub fn set_mode(mode: Mode) -> Request {
        let args: Vec<u8> = match mode {
            Mode::Off => vec![effect::OFF],
            Mode::Wave(d) => vec![effect::WAVE, d as u8],
            Mode::Breathing(c) => vec![effect::BREATHING, 1, c.0, c.1, c.2],
            Mode::BreathingDual(a, b) => {
                vec![effect::BREATHING, 2, a.0, a.1, a.2, b.0, b.1, b.2]
            }
            Mode::Spectrum => vec![effect::SPECTRUM],
            Mode::Static(c) => vec![effect::STATIC, c.0, c.1, c.2],
        };
        Request::with_size(SET_MATRIX_EFFECT, args.len() as u8, &args)
            .expect("at most 8 argument bytes")
    }

    /// 03/03 `[ledClass, ledId, value]`.
    pub fn set_brightness(led: u8, value: u8) -> Request {
        legacy::set_led_brightness(LED_CLASS, led, value)
    }

    /// 03/83.
    pub fn get_brightness(led: u8) -> Request {
        legacy::get_led_brightness(LED_CLASS, led)
    }

    /// Percent (clamped to 100) → raw value as Synapse sends it to class 0x03 (ceil).
    pub fn percent_to_raw(percent: u8) -> u8 {
        (u16::from(percent.min(100)) * 255).div_ceil(100) as u8
    }

    /// Raw class-0x03 value → percent as Synapse shows it (floor). Inverse of
    /// [`percent_to_raw`] for every percent 0–100.
    pub fn raw_to_percent(raw: u8) -> u8 {
        (u16::from(raw) * 100 / 255) as u8
    }

    /// 03/89 "Get LED ID List".
    pub fn get_led_ids() -> Request {
        fixed(GET_LED_ID_LIST, &[])
    }

    /// Reply `[count, ids…]`; the count is capped at the bytes present.
    pub fn parse_led_ids(args: &[u8]) -> Result<Vec<u8>, ProtocolError> {
        need(args, 1)?;
        let n = usize::from(args[0]).min(args.len() - 1);
        Ok(args[1..=n].to_vec())
    }
}

/// Legacy LED commands address `[ledClass, ledId, …]`.
pub mod legacy {
    use super::*;

    pub fn set_led_state(led_class: u8, led: u8, on: bool) -> Request {
        fixed(SET_LED_STATE, &[led_class, led, u8::from(on)])
    }

    pub fn get_led_state(led_class: u8, led: u8) -> Request {
        fixed(GET_LED_STATE, &[led_class, led])
    }

    pub fn set_led_rgb(led_class: u8, led: u8, c: Rgb) -> Request {
        fixed(SET_LED_RGB, &[led_class, led, c.0, c.1, c.2])
    }

    pub fn get_led_rgb(led_class: u8, led: u8) -> Request {
        fixed(GET_LED_RGB, &[led_class, led])
    }

    pub fn set_led_brightness(led_class: u8, led: u8, value: u8) -> Request {
        fixed(SET_LED_BRIGHTNESS, &[led_class, led, value])
    }

    pub fn get_led_brightness(led_class: u8, led: u8) -> Request {
        fixed(GET_LED_BRIGHTNESS, &[led_class, led])
    }

    /// Third byte of a state/brightness reply.
    pub fn parse_value(args: &[u8]) -> Result<u8, ProtocolError> {
        need(args, 3)?;
        Ok(args[2])
    }

    pub fn parse_rgb(args: &[u8]) -> Result<Rgb, ProtocolError> {
        need(args, 5)?;
        Ok(Rgb(args[2], args[3], args[4]))
    }
}

#[cfg(test)]
mod tests;
