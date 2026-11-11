//! The egui window: device list on the left, grouped settings on the right.
//!
//! egui only repaints on input or when the worker calls `request_repaint`, so the
//! window costs no CPU while idle.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

use eframe::egui::{self, Color32, RichText, Ui};
use opensynapse_protocol::buttons::Mode;
use opensynapse_protocol::dpi::{self, Dpi};

use opensynapse_sys::autostart;

use crate::agent_ctl::{self, AutostartState};
use crate::backend::{self, BackendState, EventEffect};
use crate::ops::{
    self, ActionEdit, ActionKind, ButtonRow, Field, Setting, Snapshot, WriteFailure, WriteOpts,
    WriteOutcome,
};
use crate::worker::{self, DeviceRow, Job, Reply, SubMsg};

const WARN: Color32 = Color32::from_rgb(0xE0, 0x9A, 0x20);
const BAD: Color32 = Color32::from_rgb(0xE0, 0x50, 0x50);
const GOOD: Color32 = Color32::from_rgb(0x50, 0xB0, 0x60);
/// Accent colour (user choice: green).
const ACCENT: Color32 = Color32::from_rgb(0x2E, 0xA0, 0x43);

/// Use the green accent for selections, focus and links in light and dark mode.
fn apply_accent(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke.color = Color32::WHITE;
        v.hyperlink_color = ACCENT;
        v.widgets.hovered.bg_stroke.color = ACCENT;
        v.widgets.active.bg_fill = ACCENT;
    });
}

pub struct App {
    jobs: Sender<Job>,
    replies: Receiver<Reply>,
    /// Agent events (only while the agent backend is in use).
    events: Receiver<SubMsg>,
    /// `None` until the first connection attempt was answered.
    backend: Option<BackendState>,
    /// Last message from the agent's event stream.
    agent_note: Option<String>,
    /// The agent binary next to this GUI (what "Start agent" and autostart use).
    agent_exe: Option<PathBuf>,
    /// The Run-key entry, read at start and after each toggle (no polling).
    autostart: AutostartState,
    /// Result of the last autostart toggle.
    autostart_msg: Option<Result<String, String>>,
    /// Jobs sent and not yet answered.
    pending: usize,
    devices: Vec<DeviceRow>,
    scan_error: Option<String>,
    conflict: Option<String>,
    confirm_conflict: bool,
    selected: Option<String>,
    snapshot: Option<Result<Snapshot, String>>,
    last_write: Option<Result<WriteOutcome, WriteFailure>>,
    dpi_input: u16,
    polling_input: u32,
    sleep_input: u16,
    /// "Allow editing left/right click" (off at start, never remembered).
    allow_primary: bool,
    /// Action editor state per (button, mode) slot.
    button_edits: HashMap<(u8, Mode), ActionEdit>,
    /// Keys offered by the key combo, computed once.
    key_choices: Vec<(u8, String)>,
    modifier_choices: Vec<(u8, &'static str)>,
    /// Stage editor: DPI per stage, 0-based active position, storage choice.
    stage_values: Vec<u16>,
    stage_active: usize,
    stage_persistent: bool,
    /// "Per-app profiles" window (`src/app_profiles_view.rs`).
    app_profiles: crate::app_profiles_view::AppProfilesView,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_accent(&cc.egui_ctx);
        let (jobs, replies, events) = worker::spawn(cc.egui_ctx.clone());
        let agent_exe = opensynapse_sys::agent_exe().ok();
        let autostart = read_autostart(agent_exe.as_deref());
        let mut app = Self {
            jobs,
            replies,
            events,
            backend: None,
            agent_note: None,
            agent_exe,
            autostart,
            autostart_msg: None,
            pending: 0,
            devices: Vec::new(),
            scan_error: None,
            conflict: None,
            confirm_conflict: false,
            selected: None,
            snapshot: None,
            last_write: None,
            dpi_input: 800,
            polling_input: 1000,
            sleep_input: 300,
            allow_primary: false,
            button_edits: HashMap::new(),
            key_choices: ops::key_choices(),
            modifier_choices: ops::modifier_choices(),
            stage_values: Vec::new(),
            stage_active: 0,
            stage_persistent: false,
            app_profiles: Default::default(),
        };
        app.send(Job::Connect);
        app.send(Job::Scan { reselect: true });
        app
    }

    fn send(&mut self, job: Job) {
        if self.jobs.send(job).is_ok() {
            self.pending += 1;
        }
    }

    fn busy(&self) -> bool {
        self.pending > 0
    }

    fn selected_row(&self) -> Option<&DeviceRow> {
        let path = self.selected.as_ref()?;
        self.devices.iter().find(|d| &d.path == path)
    }

    fn select(&mut self, row: &DeviceRow) {
        self.selected = Some(row.path.clone());
        self.snapshot = None;
        self.last_write = None;
        if row.supported() {
            self.send(Job::Read(row.path.clone()));
        }
    }

    fn refresh(&mut self) {
        if let Some(row) = self.selected_row().cloned() {
            self.send(Job::Read(row.path));
        }
    }

    fn write(&mut self, setting: Setting) {
        self.send_write(setting, false);
    }

    /// `restore`: write back a value read from the device (see [`WriteOpts`]).
    fn send_write(&mut self, setting: Setting, restore: bool) {
        if let Some(row) = self.selected_row().cloned() {
            self.send(Job::Write {
                path: row.path,
                setting,
                opts: WriteOpts {
                    confirmed: self.confirm_conflict,
                    allow_primary: self.allow_primary,
                    restore,
                },
            });
        }
    }

    /// Writes need an idle worker and, if Razer software runs, the confirmation.
    fn may_write(&self) -> bool {
        !self.busy() && (self.conflict.is_none() || self.confirm_conflict)
    }

    /// Apply worker replies. Runs only when a repaint was requested.
    fn drain(&mut self) {
        while let Ok(reply) = self.replies.try_recv() {
            if reply.is_answer() {
                self.pending = self.pending.saturating_sub(1);
            }
            match reply {
                Reply::Backend(state) | Reply::Switched(state) => self.set_backend(state),
                Reply::Devices {
                    list,
                    conflict,
                    reselect,
                } => {
                    self.conflict = conflict;
                    match list {
                        Ok(rows) => {
                            self.scan_error = None;
                            self.devices = rows;
                        }
                        Err(e) => {
                            self.scan_error = Some(e);
                            self.devices.clear();
                        }
                    }
                    let keep = self.selected_row().cloned();
                    if keep.is_some() && !reselect {
                        // Hotplug of another device: the panel stays as it is.
                        continue;
                    }
                    let auto = || self.devices.iter().find(|d| d.supported()).cloned();
                    match keep.or_else(auto) {
                        Some(row) => self.select(&row),
                        None => {
                            self.selected = None;
                            self.snapshot = None;
                        }
                    }
                }
                Reply::Snapshot { path, result } => {
                    if self.selected.as_ref() == Some(&path) {
                        if let Ok(s) = &result {
                            self.load_inputs(s);
                        }
                        self.snapshot = Some(result);
                    }
                }
                Reply::Written {
                    path,
                    result,
                    conflict,
                } => {
                    self.conflict = conflict;
                    if self.selected.as_ref() == Some(&path) {
                        self.last_write = Some(result);
                        self.refresh();
                    }
                }
            }
        }
        while let Ok(msg) = self.events.try_recv() {
            match msg {
                SubMsg::Event(ev) => {
                    let selected = self.selected.as_deref();
                    match backend::apply_event(selected, &mut self.snapshot, ev) {
                        EventEffect::Rescan => self.send(Job::Scan { reselect: false }),
                        EventEffect::Note(n) => self.agent_note = Some(n),
                        EventEffect::Updated | EventEffect::Nothing => {}
                    }
                }
                // A stale end (the worker already left the agent) changes nothing.
                SubMsg::Ended(reason) => {
                    if self.backend.as_ref().is_some_and(BackendState::is_agent) {
                        self.send(Job::UseLocal(reason));
                    }
                }
            }
        }
    }

    /// New backend; list devices again when it serves a different set.
    fn set_backend(&mut self, state: BackendState) {
        let rescan = backend::needs_rescan(self.backend.as_ref(), &state);
        self.backend = Some(state);
        self.agent_note = None;
        if rescan {
            self.send(Job::Scan { reselect: true });
        }
    }

    /// Start the edit fields at the values just read.
    fn load_inputs(&mut self, s: &Snapshot) {
        if let Ok(d) = s.dpi {
            self.dpi_input = d.x;
        }
        if let Ok((hz, _)) = s.polling {
            self.polling_input = hz;
        }
        if let Ok(v) = s.sleep {
            self.sleep_input = v;
        }
        if let Ok(st) = &s.stages {
            self.stage_values = st.stages.iter().map(|x| x.dpi.x).collect();
            self.stage_active = st.active_index().unwrap_or(0);
        }
        self.button_edits.clear();
        for list in [&s.buttons_normal, &s.buttons_hypershift] {
            for a in list.iter().flatten() {
                self.button_edits
                    .insert((a.button, a.mode), ActionEdit::from_assignment(a));
            }
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("OpenSynapse");
            ui.separator();
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Rescan devices"))
                .clicked()
            {
                self.send(Job::Scan { reselect: true });
            }
            self.app_profiles.button(ui);
            ui.separator();
            match &self.backend {
                None => {
                    ui.label("Backend: connecting...");
                }
                Some(state) => {
                    ui.label(state.label());
                    if !state.is_agent() {
                        let start =
                            egui::Button::new(RichText::new("Start agent").color(Color32::WHITE))
                                .fill(ACCENT);
                        let can_start = !self.busy() && self.agent_exe.is_some();
                        let r = ui.add_enabled(can_start, start);
                        let r = if self.agent_exe.is_none() {
                            r.on_disabled_hover_text(format!(
                                "{} not found next to the GUI",
                                opensynapse_sys::AGENT_EXE
                            ))
                        } else {
                            r.on_hover_text("Start the background agent and switch to it")
                        };
                        if r.clicked() {
                            self.send(Job::StartAgent);
                        }
                        if ui
                            .add_enabled(!self.busy(), egui::Button::new("Reconnect to agent"))
                            .clicked()
                        {
                            self.send(Job::Connect);
                        }
                    }
                }
            }
            if self.busy() {
                ui.label("Working...");
            }
        });
        self.autostart_row(ui);
        let note = self.backend.as_ref().and_then(BackendState::note);
        for n in note.into_iter().chain(self.agent_note.as_deref()) {
            ui.label(RichText::new(n).weak());
        }
        if let Some(name) = &self.conflict {
            ui.colored_label(
                WARN,
                format!(
                    "{name} is running and may talk to the device at the same time. \
                     Close it before changing settings."
                ),
            );
            ui.checkbox(
                &mut self.confirm_conflict,
                "I understand the risk; allow writes anyway",
            );
        }
    }

    /// "Start agent at login" (per-user Run key, ADR-0006).
    fn autostart_row(&mut self, ui: &mut Ui) {
        let blocked = agent_ctl::autostart_blocked(&self.autostart, self.agent_exe.is_some());
        let other = match &self.autostart {
            AutostartState::Other(cmd) => Some(cmd.clone()),
            _ => None,
        };
        ui.horizontal(|ui| {
            let mut on = self.autostart.is_on();
            let r = ui.add_enabled(
                blocked.is_none(),
                egui::Checkbox::new(&mut on, "Start agent at login"),
            );
            let r = match &blocked {
                Some(why) => r.on_disabled_hover_text(why.as_str()),
                None => r.on_hover_text("Per-user Run key; no admin rights, no service"),
            };
            if r.changed() {
                self.set_autostart(on);
            }
            if let Some(cmd) = &other {
                ui.colored_label(WARN, agent_ctl::other_binary_note(cmd));
                if ui
                    .add_enabled(self.agent_exe.is_some(), egui::Button::new("Fix"))
                    .on_hover_text("Point autostart at the agent next to this GUI")
                    .clicked()
                {
                    self.set_autostart(true);
                }
            }
        });
        if let AutostartState::Error(e) = &self.autostart {
            ui.colored_label(BAD, e);
        }
        match &self.autostart_msg {
            Some(Ok(m)) => {
                ui.label(RichText::new(m).weak());
            }
            Some(Err(e)) => {
                ui.colored_label(BAD, e);
            }
            None => {}
        }
    }

    /// Write or remove the Run value, then read it back for the checkbox.
    fn set_autostart(&mut self, on: bool) {
        let r = match (on, &self.agent_exe) {
            (true, Some(exe)) => autostart::enable(exe).map(|()| true),
            (true, None) => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} not found next to the GUI", opensynapse_sys::AGENT_EXE),
            )),
            (false, _) => autostart::disable(),
        };
        self.autostart_msg = Some(agent_ctl::toggle_result(on, r));
        self.autostart = read_autostart(self.agent_exe.as_deref());
    }

    fn device_list(&mut self, ui: &mut Ui) {
        ui.heading("Devices");
        if let Some(e) = &self.scan_error {
            ui.colored_label(BAD, e);
        }
        if self.devices.is_empty() && self.scan_error.is_none() {
            ui.label("No Razer device with a control interface found.");
        }
        let mut clicked = None;
        for row in &self.devices {
            let selected = self.selected.as_ref() == Some(&row.path);
            let mut text = format!("{}\n{:04X}", row.name(), row.pid);
            if !row.supported() {
                text.push_str(&format!(" - protocol {} (not supported)", row.protocol));
            }
            let r = ui.add_enabled(row.supported(), egui::Button::selectable(selected, text));
            if r.clicked() {
                clicked = Some(row.clone());
            }
        }
        if let Some(row) = clicked {
            self.select(&row);
        }
    }

    fn settings(&mut self, ui: &mut Ui) {
        let Some(row) = self.selected_row().cloned() else {
            ui.label("Select a device on the left.");
            return;
        };
        ui.horizontal(|ui| {
            ui.heading(row.name());
            ui.label(format!("({:04X})", row.pid));
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Refresh"))
                .clicked()
            {
                self.refresh();
            }
        });
        self.last_write_box(ui);
        let snap = match &self.snapshot {
            None => {
                ui.label("Reading...");
                return;
            }
            Some(Err(e)) => {
                ui.colored_label(BAD, e);
                return;
            }
            Some(Ok(s)) => s.clone(),
        };
        egui::ScrollArea::both().show(ui, |ui| {
            section(ui, "Device", |ui| {
                grid(ui, "device", |ui| {
                    kv(ui, "Firmware", snap.firmware.to_string());
                    kv(ui, "Serial", field(&snap.serial, |s| s.clone()));
                    kv(
                        ui,
                        "Device mode",
                        field(&snap.mode, |(m, p)| format!("{m:?} (param {p})")),
                    );
                    kv(
                        ui,
                        "Battery",
                        field(&snap.battery, |b| format!("{}%", b.percent())),
                    );
                    kv(ui, "Charging", field(&snap.charging, |c| c.to_string()));
                });
            });
            self.dpi_section(ui, &row, &snap);
            self.polling_section(ui, &row, &snap);
            self.power_section(ui, &snap);
            self.buttons_section(ui, &snap);
        });
    }

    fn dpi_section(&mut self, ui: &mut Ui, row: &DeviceRow, snap: &Snapshot) {
        section(ui, "DPI", |ui| {
            grid(ui, "dpi", |ui| {
                kv(ui, "Current DPI", field(&snap.dpi, |d| d.to_string()));
                kv(
                    ui,
                    "Stages ([active])",
                    field(&snap.stages, ops::stages_text),
                );
            });
            let d = row.device;
            let min = d.and_then(|d| d.dpi_min).unwrap_or(100);
            let max = d.and_then(|d| d.dpi_max).unwrap_or(30000);
            let step = d.and_then(|d| d.dpi_step).unwrap_or(50);
            let g = DpiGrid {
                min: clamp_u16(min),
                max: clamp_u16(max),
                step: clamp_u16(step),
            };
            ui.horizontal(|ui| {
                ui.label("Set DPI");
                g.drag(ui, &mut self.dpi_input);
                ui.label(RichText::new(format!("{min}-{max}, step {step}; volatile")).weak());
                if ui
                    .add_enabled(self.may_write(), egui::Button::new("Apply"))
                    .clicked()
                {
                    self.write(Setting::Dpi(Dpi::uniform(self.dpi_input)));
                }
            });
            if let Ok(st) = &snap.stages {
                self.stage_editor(ui, g, ops::stage_limit(st.stages.len()));
            }
        });
    }

    /// Editable stage list: values, add/remove, active stage, Apply (04/06).
    fn stage_editor(&mut self, ui: &mut Ui, g: DpiGrid, limit: usize) {
        ui.separator();
        ui.label(RichText::new("DPI stages").strong());
        let len = self.stage_values.len();
        let mut remove = None;
        for i in 0..len {
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.stage_active, i, format!("Stage {}", i + 1));
                g.drag(ui, &mut self.stage_values[i]);
                if ui
                    .add_enabled(len > 1, egui::Button::new("Remove"))
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.stage_values.remove(i);
            if self.stage_active > i || self.stage_active >= self.stage_values.len() {
                self.stage_active = self.stage_active.saturating_sub(1);
            }
        }
        let stages = ops::stages_from(&self.stage_values, self.stage_active);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.stage_values.len() < limit,
                    egui::Button::new("Add stage"),
                )
                .clicked()
            {
                let last = self.stage_values.last().copied().unwrap_or(g.min);
                let next = last.saturating_mul(2);
                self.stage_values
                    .push(ops::snap_dpi(next, g.min, g.max, g.step));
            }
            ui.checkbox(&mut self.stage_persistent, "Persistent");
            let ok = stages.is_ok();
            if ui
                .add_enabled(self.may_write() && ok, egui::Button::new("Apply stages"))
                .clicked()
                && let Ok(stages) = stages
            {
                let storage = if self.stage_persistent {
                    dpi::Storage::Persistent
                } else {
                    dpi::Storage::Volatile
                };
                self.write(Setting::DpiStages { storage, stages });
            }
        });
        let storage = if self.stage_persistent {
            "persistent: survives power-off"
        } else {
            "volatile: lost on power-off"
        };
        ui.label(
            RichText::new(format!(
                "Up to {limit} stages; {storage}. The active stage's DPI becomes the current DPI."
            ))
            .weak(),
        );
    }

    fn buttons_section(&mut self, ui: &mut Ui, snap: &Snapshot) {
        section(ui, "Buttons", |ui| {
            for (mode, list) in [
                (Mode::Normal, &snap.buttons_normal),
                (Mode::HyperShift, &snap.buttons_hypershift),
            ] {
                if let Err(e) = list {
                    ui.colored_label(BAD, format!("{mode} mode list: n/a ({e})"));
                }
            }
            let normal = snap.buttons_normal.as_deref().unwrap_or(&[]);
            let hyper = snap.buttons_hypershift.as_deref().unwrap_or(&[]);
            let rows = ops::button_rows(normal, hyper);
            ui.checkbox(
                &mut self.allow_primary,
                "Allow editing left/right click (without left click the mouse is hard to use)",
            );
            egui::Grid::new("buttons")
                .num_columns(3)
                .spacing([16.0, 8.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label(RichText::new("Button").strong());
                    ui.label(RichText::new("Normal").strong());
                    ui.label(RichText::new("HyperShift").strong());
                    ui.end_row();
                    for row in &rows {
                        ui.label(ops::button_name(row.button));
                        for mode in [Mode::Normal, Mode::HyperShift] {
                            self.button_cell(ui, row, mode);
                        }
                        ui.end_row();
                    }
                });
            ui.label(
                RichText::new(format!(
                    "Profile {}. Apply writes one slot: read old, write, read back.",
                    ops::BUTTON_PROFILE
                ))
                .weak(),
            );
        });
    }

    /// Current action plus editor (action combo, key + modifiers, Apply) for one slot.
    fn button_cell(&mut self, ui: &mut Ui, row: &ButtonRow, mode: Mode) {
        let Some(current) = row.slot(mode) else {
            ui.label("n/a");
            return;
        };
        let slot = (row.button, mode);
        let from_device = ActionEdit::from_assignment(current);
        let mut edit = self.button_edits.get(&slot).copied().unwrap_or(from_device);
        let locked = ops::PRIMARY_BUTTONS.contains(&row.button) && !self.allow_primary;
        let mut apply = None;
        let salt = (row.button, mode as u8);
        ui.vertical(|ui| {
            ui.label(RichText::new(format!("now: {}", ops::action_text(current))).weak());
            ui.add_enabled_ui(!locked, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("action", salt))
                        .width(130.0)
                        .selected_text(edit.kind.label())
                        .show_ui(ui, |ui| {
                            for k in ActionKind::choices() {
                                // "Other" only describes what the device holds.
                                if k == ActionKind::Other && from_device.kind != k {
                                    continue;
                                }
                                ui.selectable_value(&mut edit.kind, k, k.label());
                            }
                        });
                    let new = edit.to_assignment(row.button, mode);
                    let changed = new.as_ref().is_some_and(|a| a != current);
                    if ui
                        .add_enabled(self.may_write() && changed, egui::Button::new("Apply"))
                        .clicked()
                    {
                        apply = new;
                    }
                });
                if edit.kind == ActionKind::Key {
                    ui.horizontal(|ui| {
                        let name = self
                            .key_choices
                            .iter()
                            .find(|(u, _)| *u == edit.usage)
                            .map_or_else(|| format!("{:#04x}", edit.usage), |(_, n)| n.clone());
                        egui::ComboBox::from_id_salt(("key", salt))
                            .width(70.0)
                            .selected_text(name)
                            .show_ui(ui, |ui| {
                                for (u, n) in &self.key_choices {
                                    ui.selectable_value(&mut edit.usage, *u, n);
                                }
                            });
                        for &(bit, label) in &self.modifier_choices {
                            let mut on = edit.modifiers & bit != 0;
                            if ui.checkbox(&mut on, label).changed() {
                                edit.modifiers ^= bit;
                            }
                        }
                    });
                }
            });
        });
        self.button_edits.insert(slot, edit);
        if let Some(a) = apply {
            self.write(Setting::Button(a));
        }
    }

    fn polling_section(&mut self, ui: &mut Ui, row: &DeviceRow, snap: &Snapshot) {
        section(ui, "Polling", |ui| {
            grid(ui, "polling", |ui| {
                kv(
                    ui,
                    "Polling rate",
                    field(&snap.polling, |(hz, v)| format!("{hz} Hz ({v})")),
                );
            });
            let Ok((_, variant)) = snap.polling else {
                return;
            };
            let rates: Vec<u32> = match row.device {
                Some(d) if !d.polling_rates.is_empty() => d.polling_rates.clone(),
                _ => ops::FALLBACK_POLLING_RATES.to_vec(),
            };
            ui.horizontal(|ui| {
                ui.label("Set rate");
                egui::ComboBox::from_id_salt("polling-rate")
                    .selected_text(format!("{} Hz", self.polling_input))
                    .show_ui(ui, |ui| {
                        for hz in rates {
                            ui.selectable_value(&mut self.polling_input, hz, format!("{hz} Hz"));
                        }
                    });
                if ui
                    .add_enabled(self.may_write(), egui::Button::new("Apply"))
                    .clicked()
                {
                    self.write(Setting::Polling {
                        hz: self.polling_input,
                        variant,
                    });
                }
            });
        });
    }

    fn power_section(&mut self, ui: &mut Ui, snap: &Snapshot) {
        section(ui, "Power", |ui| {
            grid(ui, "power", |ui| {
                kv(ui, "Sleep timer", field(&snap.sleep, |v| format!("{v} s")));
            });
            ui.horizontal(|ui| {
                ui.label("Set sleep timer");
                ui.add(
                    egui::DragValue::new(&mut self.sleep_input)
                        .range(1..=3600)
                        .speed(5.0)
                        .suffix(" s"),
                );
                if ui
                    .add_enabled(self.may_write(), egui::Button::new("Apply"))
                    .clicked()
                {
                    self.write(Setting::Sleep(self.sleep_input));
                }
            });
        });
    }

    /// Result of the last write, with a Restore button for the old value.
    fn last_write_box(&mut self, ui: &mut Ui) {
        let Some(last) = self.last_write.clone() else {
            return;
        };
        let restore = match &last {
            Ok(o) => {
                if o.verified() {
                    ui.colored_label(
                        GOOD,
                        format!("Changed {} -> {}. Read back: verified.", o.old, o.new),
                    );
                } else {
                    ui.colored_label(
                        BAD,
                        format!(
                            "Wrote {} but it reads back as {} (was {}).",
                            o.new, o.now, o.old
                        ),
                    );
                }
                (o.old != o.now).then(|| o.old.clone())
            }
            Err(f) => {
                ui.colored_label(BAD, format!("Write not done: {}", f.message));
                f.old.clone()
            }
        };
        if let Some(old) = restore {
            let label = format!("Restore {old}");
            if ui
                .add_enabled(self.may_write(), egui::Button::new(label))
                .clicked()
            {
                self.send_write(old, true);
            }
        }
        ui.separator();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.drain();
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("devices")
            .default_size(230.0)
            .show(ui, |ui| self.device_list(ui));
        egui::CentralPanel::default().show(ui, |ui| self.settings(ui));
        let pid = self.selected_row().map(|d| d.pid);
        self.app_profiles.window(ui.ctx(), pid);
    }
}

fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(6.0);
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong());
        add(ui);
    });
}

fn grid(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([24.0, 4.0])
        .show(ui, add);
}

fn kv(ui: &mut Ui, key: &str, value: String) {
    ui.label(key);
    ui.label(value);
    ui.end_row();
}

fn field<T>(f: &Field<T>, show: impl FnOnce(&T) -> String) -> String {
    match f {
        Ok(v) => show(v),
        Err(e) => format!("n/a ({e})"),
    }
}

/// DPI range and step from the registry.
#[derive(Clone, Copy)]
struct DpiGrid {
    min: u16,
    max: u16,
    step: u16,
}

impl DpiGrid {
    /// DragValue that snaps to the registry grid once the user lets go.
    fn drag(self, ui: &mut Ui, v: &mut u16) {
        let r = ui.add(
            egui::DragValue::new(v)
                .range(self.min..=self.max)
                .speed(f64::from(self.step) / 4.0),
        );
        if !r.dragged() && !r.has_focus() {
            *v = ops::snap_dpi(*v, self.min, self.max, self.step);
        }
    }
}

fn clamp_u16(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

/// One registry read; cheap enough for the UI thread, done only at start and
/// after a toggle.
fn read_autostart(exe: Option<&Path>) -> AutostartState {
    AutostartState::from_read(autostart::get(), exe)
}
