//! The egui window: device list on the left, grouped settings on the right.
//!
//! egui only repaints on input or when the worker calls `request_repaint`, so the
//! window costs no CPU while idle.

use std::sync::mpsc::{Receiver, Sender};

use eframe::egui::{self, Color32, RichText, Ui};
use opensynapse_protocol::dpi::Dpi;

use crate::ops::{self, Field, Setting, Snapshot, WriteFailure, WriteOutcome};
use crate::worker::{self, DeviceRow, Job, Reply};

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
    /// Jobs sent and not yet answered.
    pending: usize,
    devices: Vec<DeviceRow>,
    scan_error: Option<String>,
    conflict: Option<&'static str>,
    confirm_conflict: bool,
    selected: Option<String>,
    snapshot: Option<Result<Snapshot, String>>,
    last_write: Option<Result<WriteOutcome, WriteFailure>>,
    dpi_input: u16,
    polling_input: u32,
    sleep_input: u16,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_accent(&cc.egui_ctx);
        let (jobs, replies) = worker::spawn(cc.egui_ctx.clone());
        let mut app = Self {
            jobs,
            replies,
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
        };
        app.send(Job::Scan);
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
        self.devices.iter().find(|d| &d.entry.path == path)
    }

    fn select(&mut self, row: &DeviceRow) {
        self.selected = Some(row.entry.path.clone());
        self.snapshot = None;
        self.last_write = None;
        if row.supported() {
            self.send(Job::Read(row.entry.clone()));
        }
    }

    fn refresh(&mut self) {
        if let Some(row) = self.selected_row().cloned() {
            self.send(Job::Read(row.entry));
        }
    }

    fn write(&mut self, setting: Setting) {
        if let Some(row) = self.selected_row().cloned() {
            self.send(Job::Write {
                entry: row.entry,
                setting,
                confirmed: self.confirm_conflict,
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
            self.pending = self.pending.saturating_sub(1);
            match reply {
                Reply::Devices { list, conflict } => {
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
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("OpenSynapse");
            ui.separator();
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Rescan devices"))
                .clicked()
            {
                self.send(Job::Scan);
            }
            if self.busy() {
                ui.label("Working...");
            }
        });
        if let Some(name) = self.conflict {
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
            let selected = self.selected.as_ref() == Some(&row.entry.path);
            let mut text = format!("{}\n{:04X}", row.name(), row.entry.pid);
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
            ui.label(format!("({:04X})", row.entry.pid));
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
        egui::ScrollArea::vertical().show(ui, |ui| {
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
            section(ui, "Buttons", |ui| {
                grid(ui, "buttons", |ui| {
                    for (b, a) in &snap.buttons {
                        let text = field(a, |r| {
                            let a = &r.assignment;
                            match a.known() {
                                Some(k) => k.to_string(),
                                None => format!("function {}, data {:?}", a.function, a.data),
                            }
                        });
                        kv(ui, &format!("Button {b}"), text);
                    }
                });
                ui.label(
                    RichText::new(format!(
                        "Profile {}, normal mode. Editing comes later.",
                        ops::BUTTON_PROFILE
                    ))
                    .weak(),
                );
            });
        });
    }

    fn dpi_section(&mut self, ui: &mut Ui, row: &DeviceRow, snap: &Snapshot) {
        section(ui, "DPI", |ui| {
            grid(ui, "dpi", |ui| {
                kv(ui, "Current DPI", field(&snap.dpi, |d| d.to_string()));
                let stages = field(&snap.stages, |s| {
                    s.stages
                        .iter()
                        .map(|st| {
                            if st.id == s.active {
                                format!("[{}]", st.dpi)
                            } else {
                                st.dpi.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("  ")
                });
                kv(ui, "Stages ([active])", stages);
            });
            let d = row.device;
            let min = d.and_then(|d| d.dpi_min).unwrap_or(100);
            let max = d.and_then(|d| d.dpi_max).unwrap_or(30000);
            let step = d.and_then(|d| d.dpi_step).unwrap_or(50);
            ui.horizontal(|ui| {
                ui.label("Set DPI");
                ui.add(
                    egui::DragValue::new(&mut self.dpi_input)
                        .range(clamp_u16(min)..=clamp_u16(max))
                        .speed(f64::from(step) / 4.0),
                );
                ui.label(RichText::new(format!("{min}-{max}, step {step}; volatile")).weak());
                if ui
                    .add_enabled(self.may_write(), egui::Button::new("Apply"))
                    .clicked()
                {
                    self.write(Setting::Dpi(Dpi::uniform(self.dpi_input)));
                }
            });
        });
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
                (o.old != o.now).then_some(o.old)
            }
            Err(f) => {
                ui.colored_label(BAD, format!("Write not done: {}", f.message));
                f.old
            }
        };
        if let Some(old) = restore {
            let label = format!("Restore {old}");
            if ui
                .add_enabled(self.may_write(), egui::Button::new(label))
                .clicked()
            {
                self.write(old);
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

fn clamp_u16(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}
