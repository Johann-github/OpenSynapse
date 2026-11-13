use super::*;

const GREEN: Rgb = Rgb(0x00, 0xFF, 0x00);
const RED: Rgb = Rgb(0xFF, 0x00, 0x00);

/// (class, id, data_size, first data_size argument bytes) of an encoded request.
fn wire(r: Request) -> (u8, u8, u8, Vec<u8>) {
    let rep = r.encode(0);
    let n = usize::from(rep.data_size());
    (
        rep.class(),
        rep.command_id(),
        rep.data_size(),
        rep.args()[..n].to_vec(),
    )
}

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
fn extended_mode_layouts() {
    let p = Storage::Persistent;
    let v = Storage::Volatile;
    assert_eq!(
        wire(set_mode(p, region::LOGO, Mode::Static(GREEN))),
        (0x0F, 0x02, 9, vec![1, 4, 1, 0, 0, 1, 0x00, 0xFF, 0x00])
    );
    assert_eq!(
        wire(set_mode(v, region::ALL, Mode::Off)),
        (0x0F, 0x02, 6, vec![0, 0, 0, 0, 0, 0])
    );
    assert_eq!(
        wire(set_mode(v, region::WHEEL, Mode::Breathing(RED))),
        (0x0F, 0x02, 9, vec![0, 1, 2, 1, 0, 1, 0xFF, 0, 0])
    );
    assert_eq!(
        wire(set_mode(v, region::LOGO, Mode::BreathingDual(RED, GREEN))),
        (
            0x0F,
            0x02,
            12,
            vec![0, 4, 2, 2, 0, 2, 0xFF, 0, 0, 0, 0xFF, 0]
        )
    );
    assert_eq!(
        wire(set_mode(v, region::BACKLIGHT, Mode::Spectrum)),
        (0x0F, 0x02, 6, vec![0, 5, 3, 0, 0, 0])
    );
    assert_eq!(
        wire(set_mode(
            v,
            region::BACKLIGHT,
            Mode::Wave(WaveDirection::Two)
        )),
        (0x0F, 0x02, 6, vec![0, 5, 4, 2, 0x28, 0])
    );
}

#[test]
fn extended_mode_round_trips() {
    let modes = [
        Mode::Off,
        Mode::Static(GREEN),
        Mode::Breathing(RED),
        Mode::BreathingDual(RED, GREEN),
        Mode::Spectrum,
        Mode::Wave(WaveDirection::One),
        Mode::Wave(WaveDirection::Two),
    ];
    for m in modes {
        let r = set_mode(Storage::Persistent, region::LOGO, m).encode(0);
        assert_eq!(parse_effect(r.args()).unwrap().mode(), Some(m), "{m}");
    }
}

#[test]
fn unknown_effects_have_no_mode() {
    let mut e = Effect::from_mode(Storage::Volatile, 4, Mode::Static(GREEN));
    e.effect = effect::REACTIVE;
    assert_eq!(e.mode(), None);
    e.effect = effect::STATIC;
    e.colors.clear();
    assert_eq!(e.mode(), None);
    e.effect = effect::WAVE;
    e.flags = 7;
    assert_eq!(e.mode(), None);
}

#[test]
fn get_effect_and_brightness_layouts() {
    let (class, id, _, args) = wire(get_effect(1, 4));
    assert_eq!((class, id, &args[..2]), (0x0F, 0x82, &[1u8, 4][..]));
    assert_eq!(
        wire(set_brightness(0, region::LOGO, 204)),
        (0x0F, 0x04, 3, vec![0, 4, 204])
    );
    assert_eq!(
        wire(get_brightness(1, region::LOGO)),
        (0x0F, 0x84, 3, vec![1, 4, 0])
    );
    assert_eq!(parse_brightness(&[1, 0, 200]).unwrap(), 200);
}

#[test]
fn brightness_percent_conversion() {
    assert_eq!(percent_to_raw(0), 0);
    assert_eq!(percent_to_raw(80), 204);
    assert_eq!(percent_to_raw(50), 127);
    assert_eq!(percent_to_raw(100), 255);
    assert_eq!(percent_to_raw(150), 255);
    assert_eq!(standard::percent_to_raw(50), 128);
    assert_eq!(standard::percent_to_raw(100), 255);
    for p in 0..=100 {
        assert_eq!(raw_to_percent(percent_to_raw(p)), p, "floor {p}");
        assert_eq!(
            standard::raw_to_percent(standard::percent_to_raw(p)),
            p,
            "ceil {p}"
        );
    }
}

#[test]
fn standard_mode_layouts() {
    use standard::set_mode;
    assert_eq!(wire(set_mode(Mode::Off)), (0x03, 0x0A, 1, vec![0]));
    assert_eq!(
        wire(set_mode(Mode::Static(GREEN))),
        (0x03, 0x0A, 4, vec![6, 0, 0xFF, 0])
    );
    assert_eq!(
        wire(set_mode(Mode::Breathing(RED))),
        (0x03, 0x0A, 5, vec![3, 1, 0xFF, 0, 0])
    );
    assert_eq!(
        wire(set_mode(Mode::BreathingDual(RED, GREEN))),
        (0x03, 0x0A, 8, vec![3, 2, 0xFF, 0, 0, 0, 0xFF, 0])
    );
    assert_eq!(wire(set_mode(Mode::Spectrum)), (0x03, 0x0A, 1, vec![4]));
    assert_eq!(
        wire(set_mode(Mode::Wave(WaveDirection::One))),
        (0x03, 0x0A, 2, vec![1, 1])
    );
}

#[test]
fn standard_brightness_and_led_ids() {
    assert_eq!(
        wire(standard::set_brightness(region::BACKLIGHT, 128)),
        (0x03, 0x03, 3, vec![1, 5, 128])
    );
    assert_eq!(
        wire(standard::get_brightness(region::BACKLIGHT)),
        (0x03, 0x83, 3, vec![1, 5, 0])
    );
    assert_eq!(wire(standard::get_led_ids()).0, 0x03);
    assert_eq!(wire(standard::get_led_ids()).1, 0x89);
    assert_eq!(
        standard::parse_led_ids(&[2, 5, 4, 0, 0]).unwrap(),
        vec![5, 4]
    );
    // A count larger than the reply is capped.
    assert_eq!(standard::parse_led_ids(&[9, 5]).unwrap(), vec![5]);
    assert!(standard::parse_led_ids(&[]).is_err());
}

#[test]
fn legacy_rgb() {
    let r = legacy::set_led_rgb(1, 4, Rgb(1, 2, 3)).encode(0);
    assert_eq!((r.class(), r.command_id(), r.data_size()), (0x03, 0x01, 5));
    assert_eq!(legacy::parse_rgb(r.args()).unwrap(), Rgb(1, 2, 3));
}

#[test]
fn rgb_text() {
    assert_eq!("00ff00".parse::<Rgb>().unwrap(), GREEN);
    assert_eq!("#FF0000".parse::<Rgb>().unwrap(), RED);
    assert_eq!("0x0000ff".parse::<Rgb>().unwrap(), Rgb(0, 0, 0xFF));
    assert!("00ff0".parse::<Rgb>().is_err());
    assert!("gg0000".parse::<Rgb>().is_err());
    assert!("+1ff00".parse::<Rgb>().is_err());
    assert_eq!(Rgb(0x12, 0xAB, 0x00).to_string(), "12ab00");
}

#[test]
fn zone_names() {
    assert_eq!(region::parse("logo"), Some(4));
    assert_eq!(region::parse("Wheel"), Some(1));
    assert_eq!(region::parse("22"), Some(22));
    assert_eq!(region::parse("nose"), None);
    assert_eq!(region::name(10), "underglow");
    assert_eq!(region::name(22), "zone 22");
}

#[test]
fn mode_text() {
    assert_eq!(Mode::Static(GREEN).to_string(), "static 00ff00");
    assert_eq!(
        Mode::BreathingDual(RED, GREEN).to_string(),
        "breathing ff0000 00ff00"
    );
    assert_eq!(Mode::Wave(WaveDirection::Two).to_string(), "wave 2");
}

#[test]
fn canonical_effects() {
    // A static read back with an odd rate byte is the same static.
    let mut read = Effect::from_mode(Storage::Persistent, 4, Mode::Static(GREEN));
    read.rate = 9;
    let want = Effect::from_mode(Storage::Persistent, 4, Mode::Static(GREEN));
    assert_eq!(read.canonical(), want);
    // Unknown effects stay raw.
    read.effect = effect::REACTIVE;
    assert_eq!(read.canonical(), read);
}
