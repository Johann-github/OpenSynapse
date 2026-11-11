//! Button assignments of a per-app profile (`[buttons]`, `[hypershift.buttons]`).
//!
//! The editor model is the main panel's: [`ActionEdit`] / [`ActionKind`] from
//! `opensynapse-device` (same choices, same labels). Profile actions go through
//! `ActionEdit::from_action` / `to_action`, which use the core and export
//! mappings; nothing here builds bytes or re-derives an action. A slot that is not
//! in the profile is "unchanged": the agent leaves that button as it is.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::hash::Hash;

use eframe::egui::{self, RichText, Ui};
use opensynapse_core::{ButtonAction, ButtonId, Buttons, HyperShiftSection, Profile};
use opensynapse_device::profile_apply::action_label;
use opensynapse_device::{
    ALLOW_PRIMARY_LABEL, ActionEdit, ActionKind, PRIMARY_BUTTONS, Snapshot, VIPER_BUTTONS,
    button_name, button_rows, key_choices, modifier_choices, primary_guard,
};
use opensynapse_protocol::buttons::Mode;

/// Combo entry for a button the profile does not list.
pub(super) const UNCHANGED: &str = "Unchanged (not in profile)";

/// One button slot of the profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    /// Not in the profile: the agent leaves the button as it is.
    Unchanged,
    /// An action the editor offers.
    Set(ActionEdit),
    /// An action from the file the editor does not offer (e.g. mouse button 7);
    /// written back as read until the user picks something else.
    Kept(ButtonAction),
}

impl Slot {
    fn from_action(a: Option<&ButtonAction>) -> Self {
        let Some(a) = a else {
            return Self::Unchanged;
        };
        let e = ActionEdit::from_action(a);
        if e.kind == ActionKind::Other {
            Self::Kept(a.clone())
        } else {
            Self::Set(e)
        }
    }

    fn to_action(&self) -> Option<ButtonAction> {
        match self {
            Self::Unchanged => None,
            Self::Set(e) => e.to_action(),
            Self::Kept(a) => Some(a.clone()),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Unchanged => UNCHANGED.into(),
            Self::Set(e) => e.kind.label(),
            Self::Kept(_) => ActionKind::Other.label(),
        }
    }
}

/// Both button maps of a profile as editor slots. Equality compares the profile
/// actions they produce, so switching a combo back and forth is not an edit.
#[derive(Clone, Debug, Default)]
pub(super) struct ButtonsForm {
    normal: BTreeMap<u8, Slot>,
    hypershift: BTreeMap<u8, Slot>,
    /// The maps as loaded: left/right click may stay as they are without the opt-in.
    loaded: [Buttons; 2],
}

impl PartialEq for ButtonsForm {
    fn eq(&self, other: &Self) -> bool {
        self.buttons() == other.buttons()
    }
}

impl Eq for ButtonsForm {}

fn index(mode: Mode) -> usize {
    match mode {
        Mode::Normal => 0,
        Mode::HyperShift => 1,
    }
}

impl ButtonsForm {
    pub(super) fn from_profile(p: &Profile) -> Self {
        let hs = p
            .hypershift
            .as_ref()
            .map(|h| h.buttons.clone())
            .unwrap_or_default();
        let slots = |m: &Buttons| {
            m.iter()
                .map(|(id, a)| (id.0, Slot::from_action(Some(a))))
                .collect()
        };
        Self {
            normal: slots(&p.buttons),
            hypershift: slots(&hs),
            loaded: [p.buttons.clone(), hs],
        }
    }

    fn map(&self, mode: Mode) -> &BTreeMap<u8, Slot> {
        match mode {
            Mode::Normal => &self.normal,
            Mode::HyperShift => &self.hypershift,
        }
    }

    pub(super) fn slot(&self, mode: Mode, id: u8) -> Slot {
        self.map(mode).get(&id).cloned().unwrap_or(Slot::Unchanged)
    }

    pub(super) fn set(&mut self, mode: Mode, id: u8, slot: Slot) {
        let map = match mode {
            Mode::Normal => &mut self.normal,
            Mode::HyperShift => &mut self.hypershift,
        };
        if slot == Slot::Unchanged {
            map.remove(&id);
        } else {
            map.insert(id, slot);
        }
    }

    /// The profile maps (normal, HyperShift).
    pub(super) fn buttons(&self) -> [Buttons; 2] {
        let to = |m: &BTreeMap<u8, Slot>| -> Buttons {
            m.iter()
                .filter_map(|(id, s)| s.to_action().map(|a| (ButtonId(*id), a)))
                .collect()
        };
        [to(&self.normal), to(&self.hypershift)]
    }

    /// Assignments in the profile, both modes.
    pub(super) fn count(&self) -> usize {
        self.buttons().iter().map(BTreeMap::len).sum()
    }

    /// Write both maps into `p`; an empty HyperShift map leaves the section out.
    pub(super) fn apply_to(&self, p: &mut Profile) {
        let [normal, hs] = self.buttons();
        p.buttons = normal;
        p.hypershift = (!hs.is_empty()).then_some(HyperShiftSection { buttons: hs });
    }

    /// Left/right click changed (added, edited or removed) without the opt-in.
    pub(super) fn guard(&self, allow_primary: bool) -> Vec<String> {
        let now = self.buttons();
        let mut errs = Vec::new();
        for (mode, prefix) in [
            (Mode::Normal, "buttons"),
            (Mode::HyperShift, "hypershift.buttons"),
        ] {
            let i = index(mode);
            for b in PRIMARY_BUTTONS {
                let id = ButtonId(b);
                if now[i].get(&id) != self.loaded[i].get(&id)
                    && let Err(e) = primary_guard(b, allow_primary)
                {
                    errs.push(format!("{prefix}.{b}: {e}"));
                }
            }
        }
        errs
    }

    /// Rows to show: the device's buttons (else the Viper layout), then buttons
    /// only the profile lists, in ID order.
    pub(super) fn row_ids(&self, device: &[u8]) -> Vec<u8> {
        let mut ids: Vec<u8> = if device.is_empty() {
            VIPER_BUTTONS.to_vec()
        } else {
            device.to_vec()
        };
        let mut extra: Vec<u8> = self
            .normal
            .keys()
            .chain(self.hypershift.keys())
            .copied()
            .filter(|id| !ids.contains(id))
            .collect();
        extra.sort_unstable();
        extra.dedup();
        ids.extend(extra);
        ids
    }
}

/// Button IDs the main window read from the selected device (both modes), empty
/// when nothing was read.
pub(super) fn device_button_ids(snap: Option<&Snapshot>) -> Vec<u8> {
    let Some(s) = snap else {
        return Vec::new();
    };
    let normal = s.buttons_normal.as_deref().unwrap_or(&[]);
    let hyper = s.buttons_hypershift.as_deref().unwrap_or(&[]);
    button_rows(normal, hyper)
        .iter()
        .map(|r| r.button)
        .collect()
}

/// Key and modifier lists for the key editor, built once.
pub(super) struct KeyLists {
    keys: Vec<(u8, String)>,
    mods: Vec<(u8, &'static str)>,
}

impl Default for KeyLists {
    fn default() -> Self {
        Self {
            keys: key_choices(),
            mods: modifier_choices(),
        }
    }
}

/// Key combo plus modifier checkboxes for an [`ActionKind::Key`] edit. Shared by
/// the main panel and this editor.
pub(crate) fn key_editor(
    ui: &mut Ui,
    salt: impl Hash + Debug,
    edit: &mut ActionEdit,
    keys: &[(u8, String)],
    mods: &[(u8, &'static str)],
) {
    ui.horizontal(|ui| {
        let name = keys
            .iter()
            .find(|(u, _)| *u == edit.usage)
            .map_or_else(|| format!("{:#04x}", edit.usage), |(_, n)| n.clone());
        egui::ComboBox::from_id_salt(("key", salt))
            .width(70.0)
            .selected_text(name)
            .show_ui(ui, |ui| {
                for (u, n) in keys {
                    ui.selectable_value(&mut edit.usage, *u, n);
                }
            });
        for &(bit, label) in mods {
            let mut on = edit.modifiers & bit != 0;
            if ui.checkbox(&mut on, label).changed() {
                edit.modifiers ^= bit;
            }
        }
    });
}

/// Shown while the profile sets buttons: button assignments live in the mouse's
/// onboard memory (02/0C survives a power cycle, observed 2026-11-11), so every
/// switch to and away from this app writes it. Flash endurance is unknown.
pub(super) const FLASH_NOTE: &str = "Button changes are stored in the mouse's onboard memory and      written on every switch to and away from this app. Keep them for apps you switch to rarely.";

/// The button part of the profile editor. `device`: button IDs of the selected
/// device when it has the profile's PID, else empty.
pub(super) fn buttons_ui(
    ui: &mut Ui,
    form: &mut ButtonsForm,
    allow_primary: &mut bool,
    device: &[u8],
    lists: &KeyLists,
) {
    ui.label(RichText::new("Buttons").strong());
    ui.weak(if device.is_empty() {
        "Viper V3 HyperSpeed layout; select a device with this PID in the main window to \
         list its own buttons."
    } else {
        "Buttons of the device selected in the main window."
    });
    ui.checkbox(allow_primary, ALLOW_PRIMARY_LABEL);
    if form.count() > 0 {
        ui.colored_label(crate::app::WARN, FLASH_NOTE);
    }
    let ids = form.row_ids(device);
    grid(
        ui,
        "app_buttons_normal",
        form,
        Mode::Normal,
        &ids,
        *allow_primary,
        lists,
    );
    let set = form.buttons()[1].len();
    egui::CollapsingHeader::new(format!("HyperShift buttons ({set} set)"))
        .id_salt("app_buttons_hypershift")
        .show(ui, |ui| {
            grid(
                ui,
                "app_buttons_hs",
                form,
                Mode::HyperShift,
                &ids,
                *allow_primary,
                lists,
            );
        });
}

fn grid(
    ui: &mut Ui,
    id: &str,
    form: &mut ButtonsForm,
    mode: Mode,
    ids: &[u8],
    allow_primary: bool,
    lists: &KeyLists,
) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            for &b in ids {
                ui.label(button_name(b));
                let locked = primary_guard(b, allow_primary).is_err();
                let mut slot = form.slot(mode, b);
                ui.add_enabled_ui(!locked, |ui| slot_ui(ui, (mode as u8, b), &mut slot, lists));
                form.set(mode, b, slot);
                ui.end_row();
            }
        });
}

/// Action combo ("unchanged" first, then the main panel's choices) and, for a key,
/// the key editor.
fn slot_ui(ui: &mut Ui, salt: (u8, u8), slot: &mut Slot, lists: &KeyLists) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("app_action", salt))
            .width(150.0)
            .selected_text(slot.label())
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(*slot == Slot::Unchanged, UNCHANGED)
                    .clicked()
                {
                    *slot = Slot::Unchanged;
                }
                for k in ActionKind::choices() {
                    if k == ActionKind::Other {
                        // Only describes an action read from the file.
                        if matches!(slot, Slot::Kept(_)) {
                            let _ = ui.selectable_label(true, k.label());
                        }
                        continue;
                    }
                    let on = matches!(slot, Slot::Set(e) if e.kind == k);
                    if ui.selectable_label(on, k.label()).clicked() {
                        *slot = Slot::Set(with_kind(slot, k));
                    }
                }
            });
        if let Slot::Kept(a) = slot {
            ui.weak(action_label(a));
        }
        if let Slot::Set(e) = slot
            && e.kind == ActionKind::Key
        {
            key_editor(ui, ("app", salt), e, &lists.keys, &lists.mods);
        }
    });
}

/// `kind`, keeping the key and modifiers already picked.
fn with_kind(slot: &Slot, kind: ActionKind) -> ActionEdit {
    match slot {
        Slot::Set(e) => ActionEdit { kind, ..*e },
        _ => ActionEdit::new(kind),
    }
}
