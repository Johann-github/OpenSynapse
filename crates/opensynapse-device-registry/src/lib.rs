//! Razer device identity and capability registry. See README.md.
//!
//! The TOML files in `data/devices/` are embedded at compile time and parsed on
//! first use. Schema: `data/devices/SCHEMA.md`.

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
    /// LED matrix `[rows, cols]`, total LEDs, number of regions (if known).
    Matrix {
        matrix: [u16; 2],
        leds: u16,
        regions: Option<u8>,
    },
    Chroma {
        chroma: bool,
    },
}

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
            "#,
        )
        .unwrap();
        let m = r.lookup(RAZER_VID, 0x0A79).unwrap();
        assert!(matches!(m, Match::Dongle(d) if d.pid == 0x0500));
        assert_eq!(m.protocol(), Protocol::Unknown);
        assert_eq!(
            r.devices()[0].lighting,
            Some(Lighting::Matrix {
                matrix: [1, 2],
                leds: 2,
                regions: None
            })
        );
    }
}
