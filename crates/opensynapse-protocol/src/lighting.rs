//! Lighting: new-style (class 0x0F) and legacy LED (class 0x03).
//! `vault/Protocol/Commands 2.5.md`. The Viper V3 HyperSpeed has no lighting.

use crate::command::*;
use crate::{ProtocolError, Request, fixed, need};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Effect IDs for class 0x0F (observed).
pub mod effect {
    pub const OFF: u8 = 0;
    pub const STATIC: u8 = 1;
    pub const BREATHING: u8 = 2;
    pub const SPECTRUM: u8 = 3;
    pub const WAVE: u8 = 4;
    pub const REACTIVE: u8 = 5;
}

/// Region IDs for class 0x0F (observed).
pub mod region {
    pub const ALL: u8 = 0;
    pub const WHEEL: u8 = 1;
    pub const DPI_LED: u8 = 2;
    pub const BATTERY: u8 = 3;
    pub const LOGO: u8 = 4;
    pub const BACKLIGHT: u8 = 5;
    pub const UNDERGLOW: u8 = 10;
    pub const SIDE_STRIPE: u8 = 11;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    pub profile: u8,
    pub region: u8,
    pub effect: u8,
    pub flags: u8,
    pub rate: u8,
    pub colors: Vec<Rgb>,
}

const EFFECT_HEADER: usize = 6;

/// `[profile, region, effect, flags, rate, n, n × RGB]`, data_size `6 + 3·n`.
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

pub fn get_brightness(profile: u8, region: u8) -> Request {
    fixed(GET_BRIGHTNESS, &[profile, region])
}

pub fn set_brightness(profile: u8, region: u8, value: u8) -> Request {
    fixed(SET_BRIGHTNESS, &[profile, region, value])
}

/// Reply `[profile, region, value]`.
pub fn parse_brightness(args: &[u8]) -> Result<u8, ProtocolError> {
    need(args, 3)?;
    Ok(args[2])
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
mod tests {
    use super::*;

    #[test]
    fn static_effect() {
        let e = Effect {
            profile: 1,
            region: region::LOGO,
            effect: effect::STATIC,
            flags: 0,
            rate: 0,
            colors: vec![Rgb(0xFF, 0x00, 0x80)],
        };
        let r = set_effect(&e).unwrap().encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x0F, 0x02, 9));
        assert_eq!(&r.args()[..9], &[1, 4, 1, 0, 0, 1, 0xFF, 0x00, 0x80]);
        assert_eq!(parse_effect(r.args()).unwrap(), e);
    }

    #[test]
    fn too_many_colors() {
        let e = Effect {
            profile: 0,
            region: 0,
            effect: 0,
            flags: 0,
            rate: 0,
            colors: vec![Rgb(0, 0, 0); 25],
        };
        assert!(set_effect(&e).is_err());
    }

    #[test]
    fn brightness() {
        let r = set_brightness(1, region::ALL, 200).encode(0);
        assert_eq!(&r.args()[..3], &[1, 0, 200]);
        assert_eq!(parse_brightness(&[1, 0, 200]).unwrap(), 200);
    }

    #[test]
    fn legacy_rgb() {
        let r = legacy::set_led_rgb(1, 4, Rgb(1, 2, 3)).encode(0);
        assert_eq!((r.class(), r.command_id(), r.data_size()), (0x03, 0x01, 5));
        assert_eq!(legacy::parse_rgb(r.args()).unwrap(), Rgb(1, 2, 3));
    }
}
