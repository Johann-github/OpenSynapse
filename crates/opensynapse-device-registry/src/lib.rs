//! Razer device identity and capability registry. See README.md.
//!
//! The TOML files in `data/devices/` are embedded at compile time and parsed on
//! first use. Schema: `data/devices/SCHEMA.md`.

use opensynapse_protocol::Framing;
use opensynapse_protocol::report40::Target;
use serde::Deserialize;
use std::sync::OnceLock;

/// Razer's USB vendor ID.
pub const RAZER_VID: u16 = 0x1532;

const SOURCES: [(&str, &str); 4] = [
    ("mice.toml", include_str!("../../../data/devices/mice.toml")),
    (
        "keyboards.toml",
        include_str!("../../../data/devices/keyboards.toml"),
    ),
    (
        "headsets.toml",
        include_str!("../../../data/devices/headsets.toml"),
    ),
    (
        "other.toml",
        include_str!("../../../data/devices/other.toml"),
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("{file}: {source}")]
    Parse {
        file: &'static str,
        source: toml::de::Error,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Mouse,
    Keyboard,
    Headset,
    Mousepad,
    Keypad,
    Dock,
    Controller,
    Speaker,
    Other,
}

/// Protocol family spoken on a PID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Protocol {
    #[serde(rename = "2.5")]
    V25,
    #[serde(rename = "3.0")]
    V30,
    #[serde(rename = "4.0")]
    V40,
    #[serde(rename = "unknown")]
    Unknown,
}

impl Protocol {
    /// Wire framing OpenSynapse implements for this family, `None` if it has none.
    /// 2.5 is hardware-tested; 4.0 is implemented from Synapse code only (inferred,
    /// no hardware) and addresses the base class's default target, the mouse.
    /// 3.0 is Bluetooth LE GATT, not HID, and stays unsupported.
    /// See `vault/Decisions/ADR-0015 Protocol 4.0 and 3.0 support.md`.
    pub fn framing(self) -> Option<Framing> {
        match self {
            Self::V25 => Some(Framing::V25),
            Self::V40 => Some(Framing::V40(Target::Mouse)),
            Self::V30 | Self::Unknown => None,
        }
    }

    /// True if OpenSynapse can talk to a PID of this family. The one owner of
    /// the "supported protocol" decision; CLI, agent and GUI ask here.
    pub fn is_supported(self) -> bool {
        self.framing().is_some()
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::V25 => "2.5",
            Self::V30 => "3.0",
            Self::V40 => "4.0",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Connection {
    Usb,
    Dongle,
    Bluetooth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Evidence {
    SynapseManifest,
    SynapseJs,
    Openrazer,
    Inferred,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Lighting {
    /// LED matrix `[rows, cols]`, total LEDs, number of regions (if known) and the
    /// zone (region) IDs from the device manifest (empty when unknown).
    Matrix {
        matrix: [u16; 2],
        leds: u16,
        regions: Option<u8>,
        #[serde(default)]
        zones: Vec<u8>,
    },
    Chroma {
        chroma: bool,
    },
}

/// Which lighting command family a device answers (`vault/Protocol/Lighting 2.5.md`,
/// `vault/Decisions/ADR-0014 Lighting.md`). Picked from the Synapse driver class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightingFamily {
    /// Class 0x0F: per-zone effects (0F/02, 0F/82) and brightness (0F/04, 0F/84).
    Extended,
    /// Class 0x03: whole-device matrix effect 03/0A (no GET) and LED brightness 03/03.
    Standard,
}

/// Driver classes that use the base class's 0x0F lighting (observed: no override; the
/// `NoSerial`/`NoBrightness` subclasses are inferred from their names).
const EXTENDED_CLASSES: [&str; 8] = [
    "rzDevice25",
    "rzDevice25DualLinkMouse",
    "rzDevice25DualLinkKeyboard",
    "rzDevice25WirelessKeyboard",
    "rzDevice25MikaWirelessKeyboard",
    "rzDevice25ChromaEffect",
    "rzDevice25NoSerial",
    "rzDevice25NoBrightness",
];

/// Driver classes that send the 03/0A matrix effect (observed for `LedMatrix` and
/// `Legacy`; the other two inferred from their names).
const STANDARD_CLASSES: [&str; 4] = [
    "rzDevice25LedMatrix",
    "rzDevice25LedMatrixSkipSetEffect",
    "rzDevice25Legacy",
    "rzDevice25LegacyNoBrightness",
];

/// Classes without a brightness command (inferred from the name).
const NO_BRIGHTNESS_CLASSES: [&str; 2] = ["rzDevice25NoBrightness", "rzDevice25LegacyNoBrightness"];

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(u16),
    Many(Vec<u16>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub pid: u16,
    pub connection: Connection,
    pub protocol: Option<Protocol>,
    pub synapse_class: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub synapse_product_id: Option<u32>,
    pub kind: Kind,
    pub protocol: Protocol,
    #[serde(default)]
    pub connection: Vec<Connection>,
    dongle_pid: Option<OneOrMany>,
    pub bluetooth_pid: Option<u16>,
    #[serde(default)]
    pub dual_link: bool,
    #[serde(default)]
    pub laptop: bool,
    pub transaction_id: Option<u8>,
    pub dpi_min: Option<u32>,
    pub dpi_max: Option<u32>,
    pub dpi_step: Option<u32>,
    #[serde(default)]
    pub polling_rates: Vec<u32>,
    #[serde(default)]
    pub battery: bool,
    pub lighting: Option<Lighting>,
    pub synapse_class: Option<String>,
    pub synapse4: Option<bool>,
    pub evidence: Evidence,
    /// Hardware-tested with OpenSynapse; read through [`Device::hardware_tested`].
    #[serde(default)]
    tested: bool,
    #[serde(default)]
    pub variants: Vec<Variant>,
}

impl Device {
    /// Receiver PIDs, if different from `pid`.
    pub fn dongle_pids(&self) -> &[u16] {
        match &self.dongle_pid {
            None => &[],
            Some(OneOrMany::One(p)) => std::slice::from_ref(p),
            Some(OneOrMany::Many(v)) => v,
        }
    }

    /// True if OpenSynapse has been tested on this product with real hardware
    /// (`tested = true` in `data/devices`). Writes to other devices are allowed but
    /// unverified; front-ends warn before them.
    pub fn hardware_tested(&self) -> bool {
        self.tested
    }

    /// Lighting family OpenSynapse may write, or `None` (no lighting, non-Chroma,
    /// laptop, or a driver class whose lighting commands are unknown).
    pub fn lighting_family(&self) -> Option<LightingFamily> {
        let Some(Lighting::Matrix { leds, .. }) = &self.lighting else {
            return None;
        };
        if *leds == 0 || self.laptop || self.protocol != Protocol::V25 {
            return None;
        }
        let class = self.synapse_class.as_deref()?;
        if EXTENDED_CLASSES.contains(&class) {
            Some(LightingFamily::Extended)
        } else if STANDARD_CLASSES.contains(&class) {
            Some(LightingFamily::Standard)
        } else {
            None
        }
    }

    /// Zone IDs from the device manifest (empty when unknown).
    pub fn lighting_zones(&self) -> &[u8] {
        match &self.lighting {
            Some(Lighting::Matrix { zones, .. }) => zones,
            _ => &[],
        }
    }

    /// False for driver classes without a brightness command.
    pub fn lighting_brightness(&self) -> bool {
        self.lighting_family().is_some()
            && !self
                .synapse_class
                .as_deref()
                .is_some_and(|c| NO_BRIGHTNESS_CLASSES.contains(&c))
    }

    /// True if the DPI value fits the sensor's range and step (when known).
    pub fn dpi_valid(&self, dpi: u32) -> bool {
        let (Some(min), Some(max)) = (self.dpi_min, self.dpi_max) else {
            return true;
        };
        let step_ok = match self.dpi_step {
            Some(step) => (dpi - min.min(dpi)).is_multiple_of(step),
            None => true,
        };
        (min..=max).contains(&dpi) && step_ok
    }
}

/// How a PID matched an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match<'a> {
    /// The entry's primary PID.
    Primary(&'a Device),
    /// One of the entry's alternate PIDs.
    Variant(&'a Device, &'a Variant),
    /// A receiver PID listed in `dongle_pid` (no per-PID details known).
    Dongle(&'a Device),
}

impl<'a> Match<'a> {
    pub fn device(&self) -> &'a Device {
        match *self {
            Self::Primary(d) | Self::Dongle(d) | Self::Variant(d, _) => d,
        }
    }

    /// Protocol family on the matched PID.
    pub fn protocol(&self) -> Protocol {
        match *self {
            Self::Primary(d) => d.protocol,
            Self::Variant(d, v) => v.protocol.unwrap_or(d.protocol),
            Self::Dongle(_) => Protocol::Unknown,
        }
    }
}

pub struct Registry {
    devices: Vec<Device>,
}

#[derive(Deserialize)]
struct File {
    device: Vec<Device>,
}

impl Registry {
    /// The embedded registry, parsed once per process.
    pub fn embedded() -> &'static Registry {
        static REG: OnceLock<Registry> = OnceLock::new();
        REG.get_or_init(|| Self::parse_embedded().expect("embedded device data is valid"))
    }

    fn parse_embedded() -> Result<Self, RegistryError> {
        let mut devices = Vec::new();
        for (file, text) in SOURCES {
            devices.extend(Self::parse(file, text)?.devices);
        }
        Ok(Self { devices })
    }

    /// Parse one TOML document in the `data/devices` schema.
    pub fn parse(file: &'static str, text: &str) -> Result<Self, RegistryError> {
        let f: File =
            toml::from_str(text).map_err(|source| RegistryError::Parse { file, source })?;
        Ok(Self { devices: f.device })
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Look up a VID/PID: primary PIDs first, then variants, then receiver PIDs.
    pub fn lookup(&self, vid: u16, pid: u16) -> Option<Match<'_>> {
        let vid_ok = |d: &&Device| d.vid == vid;
        let ds = || self.devices.iter().filter(vid_ok);
        ds().find(|d| d.pid == pid)
            .map(Match::Primary)
            .or_else(|| {
                ds().find_map(|d| {
                    d.variants
                        .iter()
                        .find(|v| v.pid == pid)
                        .map(|v| Match::Variant(d, v))
                })
            })
            .or_else(|| {
                ds().find(|d| d.dongle_pids().contains(&pid))
                    .map(Match::Dongle)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_parses() {
        let r = Registry::parse_embedded().unwrap();
        assert_eq!(r.devices().len(), 448);
    }

    #[test]
    fn primary_pids_unique() {
        let r = Registry::embedded();
        let mut pids: Vec<_> = r.devices().iter().map(|d| (d.vid, d.pid)).collect();
        pids.sort_unstable();
        let n = pids.len();
        pids.dedup();
        assert_eq!(pids.len(), n);
    }

    #[test]
    fn viper_v3_hyperspeed() {
        let m = Registry::embedded().lookup(RAZER_VID, 0x00B8).unwrap();
        let Match::Primary(d) = m else {
            panic!("expected primary match, got {m:?}")
        };
        assert_eq!(d.name, "Razer Viper V3 HyperSpeed");
        assert_eq!(d.kind, Kind::Mouse);
        assert_eq!(m.protocol(), Protocol::V25);
        assert_eq!(
            (d.dpi_min, d.dpi_max, d.dpi_step),
            (Some(100), Some(30000), Some(50))
        );
        assert_eq!(d.polling_rates, [125, 500, 1000]);
        assert!(d.battery && d.dual_link);
        assert_eq!(d.lighting, Some(Lighting::Chroma { chroma: false }));
        assert!(d.dpi_valid(800) && d.dpi_valid(30000));
        assert!(!d.dpi_valid(825) && !d.dpi_valid(50) && !d.dpi_valid(30050));
        assert!(d.hardware_tested());
    }

    #[test]
    fn only_the_viper_is_hardware_tested() {
        let r = Registry::embedded();
        let tested: Vec<u16> = r
            .devices()
            .iter()
            .filter(|d| d.hardware_tested())
            .map(|d| d.pid)
            .collect();
        assert_eq!(tested, [0x00B8]);
        let basilisk = r.lookup(RAZER_VID, 0x00B9).unwrap().device();
        assert!(!basilisk.hardware_tested());
    }

    #[test]
    fn supported_protocols_and_framing() {
        assert_eq!(Protocol::V25.framing(), Some(Framing::V25));
        assert_eq!(Protocol::V40.framing(), Some(Framing::V40(Target::Mouse)));
        assert_eq!(Protocol::V30.framing(), None);
        assert_eq!(Protocol::Unknown.framing(), None);
        assert!(Protocol::V25.is_supported() && Protocol::V40.is_supported());
        assert!(!Protocol::V30.is_supported() && !Protocol::Unknown.is_supported());
    }

    #[test]
    fn untested_4_0_device_parses_and_is_flagged_untested() {
        let r = Registry::parse(
            "test",
            r#"
            [[device]]
            name = "Test 4.0 Mouse"
            vid = 0x1532
            pid = 0x0F00
            kind = "mouse"
            protocol = "4.0"
            synapse_class = "rzDevice40"
            evidence = "inferred"
            "#,
        )
        .unwrap();
        let m = r.lookup(RAZER_VID, 0x0F00).unwrap();
        assert_eq!(m.protocol(), Protocol::V40);
        assert!(m.protocol().is_supported());
        assert!(!m.device().hardware_tested());
    }

    #[test]
    fn variant_and_dongle_lookup() {
        let r = Registry::embedded();
        let with_variant = r.devices().iter().find(|d| !d.variants.is_empty()).unwrap();
        let v = &with_variant.variants[0];
        match r.lookup(RAZER_VID, v.pid).unwrap() {
            Match::Variant(d, mv) => {
                assert_eq!(d.pid, with_variant.pid);
                assert_eq!(mv, v);
            }
            other => panic!("expected variant match, got {other:?}"),
        }
        assert!(r.lookup(RAZER_VID, 0xFFFF).is_none());
        assert!(r.lookup(0x046D, 0x00B8).is_none());
    }

    #[test]
    fn dongle_lookup_and_optional_regions() {
        let r = Registry::parse(
            "test",
            r#"
            [[device]]
            name = "Test Headset"
            vid = 0x1532
            pid = 0x0500
            kind = "headset"
            protocol = "unknown"
            dongle_pid = [0x0A78, 0x0A79]
            lighting = { matrix = [1, 2], leds = 2 }
            evidence = "inferred"
            tested = false
            "#,
        )
        .unwrap();
        let m = r.lookup(RAZER_VID, 0x0A79).unwrap();
        assert!(matches!(m, Match::Dongle(d) if d.pid == 0x0500));
        assert_eq!(m.protocol(), Protocol::Unknown);
        assert!(!m.protocol().is_supported());
        assert_eq!(
            r.devices()[0].lighting,
            Some(Lighting::Matrix {
                matrix: [1, 2],
                leds: 2,
                regions: None,
                zones: vec![],
            })
        );
    }

    #[test]
    fn lighting_families() {
        let r = Registry::embedded();
        let viper = r.lookup(RAZER_VID, 0x00B8).unwrap().device();
        assert_eq!(viper.lighting_family(), None);
        assert!(!viper.lighting_brightness());
        let of = |class: &str| {
            r.devices()
                .iter()
                .filter(|d| d.synapse_class.as_deref() == Some(class))
                .find(|d| matches!(d.lighting, Some(Lighting::Matrix { leds: 1.., .. })))
                .unwrap_or_else(|| panic!("a lit {class} device"))
        };
        let basilisk = of("rzDevice25");
        assert_eq!(basilisk.lighting_family(), Some(LightingFamily::Extended));
        assert!(basilisk.lighting_brightness());
        let matrix = of("rzDevice25LedMatrix");
        assert_eq!(matrix.lighting_family(), Some(LightingFamily::Standard));
        let no_bright = of("rzDevice25LegacyNoBrightness");
        assert_eq!(no_bright.lighting_family(), Some(LightingFamily::Standard));
        assert!(!no_bright.lighting_brightness());
        // Unknown lighting classes and laptops are refused.
        assert_eq!(of("rzDevice25Noa").lighting_family(), None);
        assert!(
            r.devices()
                .iter()
                .filter(|d| d.laptop)
                .all(|d| d.lighting_family().is_none())
        );
    }

    #[test]
    fn manifest_zones() {
        let r = Registry::embedded();
        let with_zones = r
            .devices()
            .iter()
            .filter(|d| !d.lighting_zones().is_empty())
            .count();
        assert_eq!(with_zones, 99);
        let zoned = r
            .devices()
            .iter()
            .find(|d| d.lighting_zones() == [1, 4, 10])
            .expect("a wheel + logo + underglow device");
        assert_eq!(zoned.kind, Kind::Mouse);
    }
}
