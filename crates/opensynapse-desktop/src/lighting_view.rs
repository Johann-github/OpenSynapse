//! Lighting section of the device panel ([[Decisions/ADR-0014 Lighting]]).
//!
//! Shown only for extended-family (0x0F) devices; the write goes through the same
//! guarded write as DPI (agent `Write` or in-process). Untested on hardware.

use eframe::egui::{self, RichText, Ui};
use opensynapse_device_registry::{Device, LightingFamily};
use opensynapse_protocol::lighting::{Mode, Rgb, Storage, WaveDirection, region};

use crate::ops::{self, Setting};

/// Effects the GUI offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectChoice {
    Off,
    Static,
    Breathing,
    BreathingDual,
    Spectrum,
    Wave,
}

impl EffectChoice {
    const ALL: [Self; 6] = [
        Self::Static,
        Self::Breathing,
        Self::BreathingDual,
        Self::Spectrum,
        Self::Wave,
        Self::Off,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Static => "Static colour",
            Self::Breathing => "Breathing",
            Self::BreathingDual => "Breathing (two colours)",
            Self::Spectrum => "Spectrum cycling",
            Self::Wave => "Wave",
        }
    }

    fn colours(self) -> usize {
        match self {
            Self::Static | Self::Breathing => 1,
            Self::BreathingDual => 2,
            Self::Off | Self::Spectrum | Self::Wave => 0,
        }
    }
}

/// Editor state; starts at static accent green, full brightness, volatile.
#[derive(Clone, Debug)]
pub struct LightingEdit {
    pub effect: EffectChoice,
    pub colour: [u8; 3],
    pub colour2: [u8; 3],
    pub brightness: u8,
    pub zone: u8,
    pub persistent: bool,
    pub wave_two: bool,
}

impl Default for LightingEdit {
    fn default() -> Self {
        Self {
            effect: EffectChoice::Static,
            colour: opensynapse_icon::ACCENT,
            colour2: [0xFF, 0xFF, 0xFF],
            brightness: 100,
            zone: region::ALL,
            persistent: false,
            wave_two: false,
        }
    }
}

/// Zones the editor offers: the registry's, or `all` when it lists none.
pub fn zone_choices(d: &Device) -> Vec<u8> {
    match d.lighting_zones() {
        [] => vec![region::ALL],
        z => z.to_vec(),
    }
}

impl LightingEdit {
    pub fn mode(&self) -> Mode {
        let rgb = |[r, g, b]: [u8; 3]| Rgb(r, g, b);
        match self.effect {
            EffectChoice::Off => Mode::Off,
            EffectChoice::Static => Mode::Static(rgb(self.colour)),
            EffectChoice::Breathing => Mode::Breathing(rgb(self.colour)),
            EffectChoice::BreathingDual => Mode::BreathingDual(rgb(self.colour), rgb(self.colour2)),
            EffectChoice::Spectrum => Mode::Spectrum,
            EffectChoice::Wave => Mode::Wave(if self.wave_two {
                WaveDirection::Two
            } else {
                WaveDirection::One
            }),
        }
    }

    /// The setting Apply writes (brightness only where the device has it).
    pub fn setting(&self, d: &Device) -> Setting {
        let storage = if self.persistent {
            Storage::Persistent
        } else {
            Storage::Volatile
        };
        let bright = d.lighting_brightness().then_some(self.brightness);
        ops::lighting_setting(storage, self.zone, self.mode(), bright)
    }

    /// Keep the zone valid for `d` (first choice when the device changed).
    fn fit_zone(&mut self, d: &Device) {
        let zones = zone_choices(d);
        if !zones.contains(&self.zone) {
            self.zone = zones[0];
        }
    }

    /// Draw the section for `d`; returns the setting to write when Apply was clicked.
    /// Nothing for devices without lighting.
    pub fn show(&mut self, ui: &mut Ui, d: &Device, may_write: bool) -> Option<Setting> {
        let family = d.lighting_family()?;
        let mut apply = None;
        crate::app::section(ui, "Lighting", |ui| {
            if family == LightingFamily::Standard {
                ui.label(
                    "This device uses the whole-device 03/0A effect, which cannot be read \
                     back; set it with the CLI (`opensynapse lighting set`).",
                );
                return;
            }
            self.fit_zone(d);
            ui.horizontal(|ui| {
                ui.label("Effect");
                egui::ComboBox::from_id_salt("light-effect")
                    .selected_text(self.effect.label())
                    .show_ui(ui, |ui| {
                        for e in EffectChoice::ALL {
                            ui.selectable_value(&mut self.effect, e, e.label());
                        }
                    });
                ui.label("Zone");
                egui::ComboBox::from_id_salt("light-zone")
                    .selected_text(region::name(self.zone))
                    .show_ui(ui, |ui| {
                        for z in zone_choices(d) {
                            ui.selectable_value(&mut self.zone, z, region::name(z));
                        }
                    });
            });
            ui.horizontal(|ui| {
                let n = self.effect.colours();
                if n >= 1 {
                    ui.label("Colour");
                    egui::color_picker::color_edit_button_srgb(ui, &mut self.colour);
                }
                if n >= 2 {
                    egui::color_picker::color_edit_button_srgb(ui, &mut self.colour2);
                }
                if self.effect == EffectChoice::Wave {
                    ui.checkbox(&mut self.wave_two, "Reverse direction");
                }
            });
            if d.lighting_brightness() {
                ui.horizontal(|ui| {
                    ui.label("Brightness");
                    ui.add(egui::Slider::new(&mut self.brightness, 0..=100).suffix(" %"));
                });
            }
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.persistent, "Save on device");
                ui.label(
                    RichText::new(format!(
                        "{}; untested on real hardware",
                        if self.persistent {
                            "storage byte 1"
                        } else {
                            "volatile, lost on unplug"
                        }
                    ))
                    .weak(),
                );
                if ui
                    .add_enabled(may_write, egui::Button::new("Apply"))
                    .clicked()
                {
                    apply = Some(self.setting(d));
                }
            });
        });
        apply
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensynapse_device_registry::Registry;
    use opensynapse_protocol::lighting;

    fn device(f: impl Fn(&Device) -> bool) -> &'static Device {
        Registry::embedded()
            .devices()
            .iter()
            .find(|d| f(d))
            .unwrap()
    }

    #[test]
    fn default_is_static_accent_green() {
        let [r, g, b] = opensynapse_icon::ACCENT;
        let e = LightingEdit::default();
        assert_eq!(e.mode(), Mode::Static(Rgb(r, g, b)));
        assert!(!e.persistent);
    }

    #[test]
    fn setting_bytes_follow_the_choices() {
        let mouse = device(|d| {
            d.lighting_family() == Some(LightingFamily::Extended)
                && d.lighting_zones() == [1, 4, 10]
                && d.lighting_brightness()
        });
        let mut e = LightingEdit::default();
        e.fit_zone(mouse);
        assert_eq!(e.zone, 1, "first registry zone");
        e.zone = 4;
        e.effect = EffectChoice::BreathingDual;
        e.colour = [0xFF, 0, 0];
        e.colour2 = [0, 0, 0xFF];
        e.brightness = 80;
        e.persistent = true;
        let Setting::Lighting { effect, brightness } = e.setting(mouse) else {
            panic!("lighting setting")
        };
        let r = lighting::set_effect(&effect).unwrap().encode(0);
        assert_eq!(&r.args()[..12], &[1, 4, 2, 2, 0, 2, 0xFF, 0, 0, 0, 0, 0xFF]);
        assert_eq!(brightness, Some(204));
    }

    #[test]
    fn zones_fall_back_to_all() {
        let unzoned = device(|d| {
            d.lighting_family() == Some(LightingFamily::Extended) && d.lighting_zones().is_empty()
        });
        assert_eq!(zone_choices(unzoned), [region::ALL]);
    }
}
