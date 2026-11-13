//! `lighting get|set`: hardware lighting effects ([[Decisions/ADR-0014 Lighting]]).
//!
//! The registry decides the command family (`Device::lighting_family`); report bytes
//! come from `opensynapse_protocol::lighting`. Untested on hardware: no RGB device is
//! `tested = true`, so the untested-device warning precedes every write.

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use opensynapse_device_registry::{Device, LightingFamily};
use opensynapse_protocol::Request;
use opensynapse_protocol::lighting::{self, Mode, Rgb, Storage, WaveDirection, region};

use crate::{Target, WriteOpts};

#[derive(Subcommand)]
pub enum LightingAction {
    /// Show the effect and brightness of a zone (brightness only for 03/0A devices,
    /// whose effect cannot be read).
    Get {
        /// Zone name (logo, wheel, backlight, underglow, side-stripe, …) or ID.
        /// Default: the device's first known zone.
        #[arg(long)]
        zone: Option<String>,
        /// Read the saved copy (storage byte 1) instead of the volatile one (0).
        #[arg(long)]
        persistent: bool,
    },
    /// Set a hardware effect. Without --apply this only shows what would be sent.
    Set {
        #[command(flatten)]
        effect: EffectArgs,
        /// Zone name or ID; default `all` (0).
        #[arg(long)]
        zone: Option<String>,
        /// Also set the brightness, in percent (0-100).
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
        brightness: Option<u8>,
        /// Save on the device (storage byte 1) instead of volatile (0).
        #[arg(long)]
        persistent: bool,
        #[command(flatten)]
        write: WriteOpts,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum EffectKind {
    Off,
    Static,
    Breathing,
    Spectrum,
    Wave,
}

#[derive(Args, Clone, Debug)]
pub struct EffectArgs {
    /// Effect to set.
    effect: EffectKind,
    /// Colours as hex (`00ff00`): static takes one, breathing one or two.
    #[arg(value_parser = parse_rgb)]
    colors: Vec<Rgb>,
    /// wave only: direction byte 1 or 2 (which one is "left" is unknown).
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    direction: Option<u8>,
}

fn parse_rgb(s: &str) -> Result<Rgb, String> {
    s.parse()
        .map_err(|e: opensynapse_protocol::ProtocolError| e.to_string())
}

/// The [`Mode`] the arguments ask for.
pub(crate) fn requested_mode(a: &EffectArgs) -> Result<Mode> {
    if a.direction.is_some() && a.effect != EffectKind::Wave {
        bail!("--direction only applies to `wave`");
    }
    let colors = |min: usize, max: usize| -> Result<()> {
        let n = a.colors.len();
        if n < min || n > max {
            let want = match (min, max) {
                (0, 0) => "no colour".to_string(),
                (a, b) if a == b => format!("{a} colour"),
                (a, b) => format!("{a} or {b} colours"),
            };
            bail!("{:?} takes {want}, got {n}", a.effect);
        }
        Ok(())
    };
    Ok(match a.effect {
        EffectKind::Off => {
            colors(0, 0)?;
            Mode::Off
        }
        EffectKind::Static => {
            colors(1, 1)?;
            Mode::Static(a.colors[0])
        }
        EffectKind::Breathing => {
            colors(1, 2)?;
            match a.colors[..] {
                [c] => Mode::Breathing(c),
                [c1, c2] => Mode::BreathingDual(c1, c2),
                _ => unreachable!("checked above"),
            }
        }
        EffectKind::Spectrum => {
            colors(0, 0)?;
            Mode::Spectrum
        }
        EffectKind::Wave => {
            colors(0, 0)?;
            Mode::Wave(match a.direction.unwrap_or(1) {
                2 => WaveDirection::Two,
                _ => WaveDirection::One,
            })
        }
    })
}

/// CLI arguments that set `mode` again.
pub(crate) fn mode_args(mode: Mode) -> String {
    match mode {
        Mode::Wave(d) => format!("wave --direction {}", d as u8),
        m => m.to_string(),
    }
}

/// The device's lighting family, or why lighting is refused.
pub(crate) fn family(device: Option<&Device>) -> Result<LightingFamily> {
    let Some(d) = device else {
        bail!("device is not in the registry; its lighting commands are unknown")
    };
    d.lighting_family().ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no lighting OpenSynapse can set (non-Chroma, or its lighting commands \
             are unknown)",
            d.name
        )
    })
}

/// Zone ID from `--zone`; must be one the registry lists, unless `--force` or the
/// registry lists none.
pub(crate) fn zone_arg(arg: &str, device: &Device, force: bool) -> Result<u8> {
    let Some(z) = region::parse(arg) else {
        bail!(
            "unknown zone `{arg}` (all, logo, wheel, backlight, underglow, side-stripe, … or an ID)"
        )
    };
    let known = device.lighting_zones();
    if z != region::ALL && !known.is_empty() && !known.contains(&z) && !force {
        bail!(
            "{} is not a zone of this device ({}); use --force to override",
            region::name(z),
            zone_list(known)
        );
    }
    Ok(z)
}

/// `--zone` value for `z`: its name, or the number when it has none.
fn zone_token(z: u8) -> String {
    let n = region::name(z);
    if n.starts_with("zone ") {
        z.to_string()
    } else {
        n
    }
}

fn zone_list(zones: &[u8]) -> String {
    zones
        .iter()
        .map(|&z| region::name(z))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Zone used for reads: the requested one, except `all`, which Synapse never reads;
/// then the first registry zone (or 0 when none is known).
pub(crate) fn read_zone(zone: u8, device: &Device) -> u8 {
    if zone == region::ALL {
        device.lighting_zones().first().copied().unwrap_or(zone)
    } else {
        zone
    }
}

fn storage(persistent: bool) -> Storage {
    if persistent {
        Storage::Persistent
    } else {
        Storage::Volatile
    }
}

pub(crate) fn run(dev: &mut Target, action: LightingAction) -> Result<()> {
    let fam = family(dev.device)?;
    let device = dev.device.expect("family() checked the registry entry");
    match action {
        LightingAction::Get { zone, persistent } => {
            let zone = match zone {
                Some(z) => zone_arg(&z, device, true)?,
                None => read_zone(region::ALL, device),
            };
            get(dev, fam, device, zone, persistent)
        }
        LightingAction::Set {
            effect,
            zone,
            brightness,
            persistent,
            write,
        } => {
            let mode = requested_mode(&effect)?;
            let zone = match zone {
                Some(z) => zone_arg(&z, device, write.force)?,
                None => region::ALL,
            };
            if brightness.is_some() && !device.lighting_brightness() {
                bail!("{} has no brightness command", device.name);
            }
            match fam {
                LightingFamily::Extended => {
                    set_extended(dev, device, zone, mode, brightness, persistent, write)
                }
                LightingFamily::Standard => {
                    if persistent {
                        bail!("--persistent does not apply: 03/0A has no storage byte");
                    }
                    set_standard(dev, zone, mode, brightness, write)
                }
            }
        }
    }
}

fn get(
    dev: &mut Target,
    fam: LightingFamily,
    device: &Device,
    zone: u8,
    persistent: bool,
) -> Result<()> {
    if !device.lighting_zones().is_empty() {
        println!("Zones: {}", zone_list(device.lighting_zones()));
    }
    match fam {
        LightingFamily::Extended => {
            let p = storage(persistent) as u8;
            let e = dev.query(&lighting::get_effect(p, zone), lighting::parse_effect)?;
            match e.mode() {
                Some(m) => println!("Effect ({}): {m}", region::name(zone)),
                None => println!(
                    "Effect ({}): other (effect {}, flags {}, rate {}, colours {:?})",
                    region::name(zone),
                    e.effect,
                    e.flags,
                    e.rate,
                    e.colors.iter().map(Rgb::to_string).collect::<Vec<_>>()
                ),
            }
            if device.lighting_brightness() {
                let raw = dev.query(
                    &lighting::get_brightness(p, zone),
                    lighting::parse_brightness,
                )?;
                println!(
                    "Brightness ({}): {}% (raw {raw})",
                    region::name(zone),
                    lighting::raw_to_percent(raw)
                );
            }
        }
        LightingFamily::Standard => {
            if persistent {
                bail!("--persistent does not apply: 03/0A has no storage byte");
            }
            println!("Effect: cannot be read (03/0A has no GET).");
            if device.lighting_brightness() {
                let led = standard_led(zone);
                let raw = dev.query(
                    &lighting::standard::get_brightness(led),
                    lighting::legacy::parse_value,
                )?;
                println!(
                    "Brightness ({}): {}% (raw {raw})",
                    region::name(led),
                    lighting::standard::raw_to_percent(raw)
                );
            }
        }
    }
    Ok(())
}

/// LED ID for standard-family brightness: the zone, or backlight for `all`
/// (inferred, `vault/Protocol/Lighting 2.5.md`).
fn standard_led(zone: u8) -> u8 {
    if zone == region::ALL {
        region::BACKLIGHT
    } else {
        zone
    }
}

/// Old effect and brightness of an extended zone, if they can be read.
struct Before {
    mode: Option<Mode>,
    raw: lighting::Effect,
    brightness: Option<u8>,
}

fn set_extended(
    dev: &mut Target,
    device: &Device,
    zone: u8,
    mode: Mode,
    brightness: Option<u8>,
    persistent: bool,
    write: WriteOpts,
) -> Result<()> {
    let st = storage(persistent);
    let rz = read_zone(zone, device);
    let before = read_extended(dev, st, rz, brightness.is_some());
    let reqs = extended_requests(zone, mode, brightness, st);
    let flag = if persistent { " --persistent" } else { "" };
    let (old_text, restore) = match &before {
        Ok(b) => {
            let restore = b.mode.map(|m| {
                let bright = b.brightness.map_or(String::new(), |r| {
                    format!(" --brightness {}", lighting::raw_to_percent(r))
                });
                format!(
                    "lighting set {} --zone {}{bright}{flag} --apply",
                    mode_args(m),
                    zone_token(zone)
                )
            });
            if restore.is_none() {
                println!(
                    "Note: the current effect (effect {}, flags {}, rate {}) cannot be set by \
                     the CLI; write it down to restore it later.",
                    b.raw.effect, b.raw.flags, b.raw.rate
                );
            }
            let text = b.mode.map_or("other".to_string(), |m| m.to_string());
            (text, restore)
        }
        Err(e) => {
            if write.apply && !write.force {
                bail!(
                    "cannot read the current effect of {} ({e}); without it there is no \
                     restore. Pass --force to write anyway",
                    region::name(rz)
                );
            }
            println!(
                "Note: cannot read the current effect of {} ({e}).",
                region::name(rz)
            );
            ("unknown".to_string(), None)
        }
    };
    let bright = brightness.map_or(String::new(), |p| format!(", brightness {p}%"));
    dev.write_all(
        write,
        &format!(
            "lighting ({}) {old_text} -> {mode}{bright}",
            region::name(zone)
        ),
        restore.as_deref().unwrap_or(""),
        &reqs,
    )?;
    if write.apply {
        if restore.is_none() {
            println!("To restore: not possible with the CLI (old effect unknown).");
        }
        let now = read_extended(dev, st, rz, brightness.is_some())?;
        let want = if zone == region::ALL && rz != zone {
            format!("{mode} (read from {})", region::name(rz))
        } else {
            mode.to_string()
        };
        match now.mode {
            Some(m) if m == mode => println!("Verified: effect is now {m}."),
            Some(m) => bail!("effect reads back as {m}, expected {want}"),
            None => bail!(
                "effect reads back as effect {} (flags {}), expected {want}",
                now.raw.effect,
                now.raw.flags
            ),
        }
        if let (Some(p), Some(raw)) = (brightness, now.brightness) {
            crate::verify("brightness", lighting::raw_to_percent(raw), p, "%")?;
        }
    }
    Ok(())
}

fn read_extended(dev: &mut Target, st: Storage, zone: u8, with_brightness: bool) -> Result<Before> {
    let raw = dev.query(
        &lighting::get_effect(st as u8, zone),
        lighting::parse_effect,
    )?;
    let brightness = if with_brightness {
        Some(dev.query(
            &lighting::get_brightness(st as u8, zone),
            lighting::parse_brightness,
        )?)
    } else {
        None
    };
    Ok(Before {
        mode: raw.mode(),
        raw,
        brightness,
    })
}

fn set_standard(
    dev: &mut Target,
    zone: u8,
    mode: Mode,
    brightness: Option<u8>,
    write: WriteOpts,
) -> Result<()> {
    println!(
        "Note: this device uses the 03/0A matrix effect for the whole device; it has no read \
         command, so the old effect cannot be shown or restored."
    );
    let mut reqs = vec![lighting::standard::set_mode(mode)];
    let led = standard_led(zone);
    let mut restore = String::new();
    let mut bright_text = String::new();
    if let Some(p) = brightness {
        let old = dev.query(
            &lighting::standard::get_brightness(led),
            lighting::legacy::parse_value,
        )?;
        let old_p = lighting::standard::raw_to_percent(old);
        bright_text = format!(", brightness ({}) {old_p}% -> {p}%", region::name(led));
        // Only the brightness can be restored; the effect argument is required, so
        // the restore line names the effect just written.
        restore = format!(
            "lighting set {} --zone {} --brightness {old_p} --apply",
            mode_args(mode),
            zone_token(led)
        );
        reqs.push(lighting::standard::set_brightness(
            led,
            lighting::standard::percent_to_raw(p),
        ));
    }
    dev.write_all(
        write,
        &format!("lighting effect -> {mode}{bright_text}"),
        &restore,
        &reqs,
    )?;
    if write.apply {
        println!("Effect: not verified (no read command).");
        if let Some(p) = brightness {
            let now = dev.query(
                &lighting::standard::get_brightness(led),
                lighting::legacy::parse_value,
            )?;
            crate::verify(
                "brightness",
                lighting::standard::raw_to_percent(now),
                p,
                "%",
            )?;
        }
    }
    Ok(())
}

/// Requests `lighting set` sends to an extended-family device.
pub(crate) fn extended_requests(
    zone: u8,
    mode: Mode,
    brightness: Option<u8>,
    st: Storage,
) -> Vec<Request> {
    let mut v = vec![lighting::set_mode(st, zone, mode)];
    if let Some(p) = brightness {
        v.push(lighting::set_brightness(
            st as u8,
            zone,
            lighting::percent_to_raw(p),
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Command};
    use clap::Parser;
    use opensynapse_device_registry::{Lighting, RAZER_VID, Registry};

    fn parse(argv: &[&str]) -> LightingAction {
        let argv = ["opensynapse", "lighting"]
            .into_iter()
            .chain(argv.iter().copied());
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Lighting { action } => action,
            _ => panic!("parsed wrong command"),
        }
    }

    fn mode_of(argv: &[&str]) -> Result<Mode> {
        match parse(argv) {
            LightingAction::Set { effect, .. } => requested_mode(&effect),
            LightingAction::Get { .. } => panic!("expected set"),
        }
    }

    const GREEN: Rgb = Rgb(0, 0xFF, 0);

    #[test]
    fn modes_from_arguments() {
        assert_eq!(mode_of(&["set", "off"]).unwrap(), Mode::Off);
        assert_eq!(
            mode_of(&["set", "static", "00ff00"]).unwrap(),
            Mode::Static(GREEN)
        );
        assert_eq!(
            mode_of(&["set", "breathing", "#ff0000", "00ff00"]).unwrap(),
            Mode::BreathingDual(Rgb(0xFF, 0, 0), GREEN)
        );
        assert_eq!(mode_of(&["set", "spectrum"]).unwrap(), Mode::Spectrum);
        assert_eq!(
            mode_of(&["set", "wave", "--direction", "2"]).unwrap(),
            Mode::Wave(WaveDirection::Two)
        );
        assert!(mode_of(&["set", "static"]).is_err());
        assert!(mode_of(&["set", "off", "00ff00"]).is_err());
        assert!(mode_of(&["set", "breathing", "00ff00", "00ff00", "00ff00"]).is_err());
        assert!(mode_of(&["set", "static", "00ff00", "--direction", "1"]).is_err());
        let argv = ["opensynapse", "lighting", "set", "static", "nope"];
        assert!(Cli::try_parse_from(argv).is_err());
        let argv = [
            "opensynapse",
            "lighting",
            "set",
            "off",
            "--brightness",
            "101",
        ];
        assert!(Cli::try_parse_from(argv).is_err());
    }

    #[test]
    fn restore_arguments_parse_back() {
        for m in [
            Mode::Off,
            Mode::Static(GREEN),
            Mode::Breathing(GREEN),
            Mode::BreathingDual(GREEN, Rgb(1, 2, 3)),
            Mode::Spectrum,
            Mode::Wave(WaveDirection::Two),
        ] {
            let line = format!("set {} --zone logo --apply", mode_args(m));
            let argv: Vec<&str> = line.split(' ').collect();
            assert_eq!(mode_of(&argv).unwrap(), m, "{line}");
        }
    }

    #[test]
    fn set_is_a_write_and_get_is_not() {
        let writes = |argv: &[&str]| {
            let argv = ["opensynapse", "lighting"]
                .into_iter()
                .chain(argv.iter().copied());
            Cli::try_parse_from(argv).unwrap().command.writes()
        };
        assert!(writes(&["set", "static", "00ff00", "--zone", "logo"]));
        assert!(!writes(&["get"]));
    }

    #[test]
    fn families_and_zones() {
        let reg = Registry::embedded();
        let viper = reg.lookup(RAZER_VID, 0x00B8).unwrap().device();
        let err = family(Some(viper)).unwrap_err().to_string();
        assert!(err.contains("no lighting"), "{err}");
        assert!(family(None).is_err());

        let mouse = reg
            .devices()
            .iter()
            .find(|d| {
                d.lighting_family() == Some(LightingFamily::Extended)
                    && d.lighting_zones() == [1, 4, 10]
            })
            .unwrap();
        assert_eq!(family(Some(mouse)).unwrap(), LightingFamily::Extended);
        assert_eq!(zone_arg("logo", mouse, false).unwrap(), 4);
        assert_eq!(zone_arg("all", mouse, false).unwrap(), 0);
        assert!(zone_arg("backlight", mouse, false).is_err());
        assert_eq!(zone_arg("backlight", mouse, true).unwrap(), 5);
        assert!(zone_arg("nose", mouse, true).is_err());
        assert_eq!(read_zone(0, mouse), 1);
        assert_eq!(read_zone(4, mouse), 4);
        assert_eq!(zone_token(4), "logo");
        assert_eq!(zone_token(22), "22");
        assert_eq!(region::parse(&zone_token(22)), Some(22));

        let no_zones = reg
            .devices()
            .iter()
            .find(|d| {
                d.lighting_family().is_some()
                    && matches!(&d.lighting, Some(Lighting::Matrix { zones, .. }) if zones.is_empty())
            })
            .unwrap();
        assert_eq!(zone_arg("backlight", no_zones, false).unwrap(), 5);
        assert_eq!(read_zone(0, no_zones), 0);
    }

    #[test]
    fn extended_bytes() {
        let reqs = extended_requests(4, Mode::Static(GREEN), Some(80), Storage::Volatile);
        let a = reqs[0].encode(0);
        assert_eq!((a.class(), a.command_id(), a.data_size()), (0x0F, 0x02, 9));
        assert_eq!(&a.args()[..9], &[0, 4, 1, 0, 0, 1, 0, 0xFF, 0]);
        let b = reqs[1].encode(0);
        assert_eq!((b.class(), b.command_id()), (0x0F, 0x04));
        assert_eq!(&b.args()[..3], &[0, 4, 204]);
    }
}
