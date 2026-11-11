//! Per-app profiles editor (`vault/Decisions/ADR-0007 Per-app profiles.md`): a
//! window that lists the rules of `app-profiles.toml` (application → profile
//! file), edits them and the settings of the profile a rule applies, saves both
//! files atomically and asks a running agent to reload them.
//!
//! The file formats and their checks belong to `opensynapse-core` (`AppProfiles`,
//! `Profile`); this module edits values and does the file and IPC I/O. It never
//! talks to a device: after the reload the agent applies profiles itself. Agent
//! calls run on a short-lived thread with their own connection and use the same
//! `AppProfiles` / `ReloadAppProfiles` requests as `opensynapse agent app-profiles`.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui::{self, RichText, Ui};
use opensynapse_core::app_profiles::{self, resolve_profile_path};
use opensynapse_core::validate::POLLING_RATES;
use opensynapse_core::{
    AppProfiles, AppRule, DeviceLimits, DeviceSection, DpiSection, Pid, PollingSection,
    PowerSection, Profile,
};
use opensynapse_device::profile_apply::limits_for;
use opensynapse_device_registry::{Device, RAZER_VID, Registry};
use opensynapse_ipc::Client;
use opensynapse_ipc::msg::{AppProfilesInfo, Reply, Request};

/// Sleep timer range offered by the editor (same as the main window).
const SLEEP_RANGE: std::ops::RangeInclusive<u16> = 1..=3600;

// ---------------------------------------------------------------------------
// Agent

/// What the agent said about its per-app profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AgentState {
    Asking,
    NotRunning,
    Info(AppProfilesInfo),
    Error(String),
}

/// One request on its own connection. No agent (`NotFound`) is a normal state.
fn ask_agent(req: Request) -> AgentState {
    let mut c = match Client::connect() {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return AgentState::NotRunning,
        Err(e) => return AgentState::Error(format!("cannot reach the agent: {e}")),
    };
    match c.call(req) {
        Ok(Ok(Reply::AppProfiles(i))) => AgentState::Info(i),
        Ok(Ok(other)) => AgentState::Error(format!("unexpected agent reply: {other:?}")),
        Ok(Err(e)) => AgentState::Error(e.message),
        Err(e) => AgentState::Error(format!("agent call failed: {e}")),
    }
}

/// The file the agent reads (it may run with `--app-profiles`), else the default
/// location under `APPDATA`.
fn rules_file(agent: &AgentState, appdata: Option<&str>) -> Option<PathBuf> {
    match agent {
        AgentState::Info(AppProfilesInfo {
            config: Some(c), ..
        }) => Some(PathBuf::from(c)),
        _ => opensynapse_sys::config::app_profiles_path_from(appdata),
    }
}

/// One status line about the agent.
fn agent_line(a: &AgentState) -> String {
    match a {
        AgentState::Asking => "Agent: asking...".into(),
        AgentState::NotRunning => "Agent: not running. It reads the rules when it starts.".into(),
        AgentState::Error(e) => format!("Agent: {e}"),
        AgentState::Info(i) => {
            let mut s = if i.enabled {
                format!("Agent: per-app profiles on, {} rule(s)", i.rules.len())
            } else {
                "Agent: per-app profiles off".into()
            };
            if let Some(fg) = &i.foreground {
                s.push_str(&format!("; app in front: {}", file_name(fg)));
            }
            if let Some(a) = &i.active {
                s.push_str(&format!("; active profile: {a}"));
            }
            s
        }
    }
}

/// What the agent said after `ReloadAppProfiles`.
fn reload_note(a: &AgentState) -> Result<String, String> {
    match a {
        AgentState::NotRunning => {
            Ok("The agent is not running; it reads the rules when it starts.".into())
        }
        AgentState::Info(AppProfilesInfo { error: Some(e), .. }) => {
            Err(format!("The agent rejected the file: {e}"))
        }
        AgentState::Info(i) if i.enabled => Ok(format!(
            "The agent reloaded the rules ({} rule(s)).",
            i.rules.len()
        )),
        AgentState::Info(_) => Ok("The agent reloaded; per-app profiles are off.".into()),
        AgentState::Error(e) => Err(format!("Saved, but the agent did not reload: {e}")),
        AgentState::Asking => Ok("Asking the agent to reload...".into()),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

// ---------------------------------------------------------------------------
// Files

/// Write `text` to a temporary file next to `path`, flush it, then rename it over
/// `path` (replaces atomically on the same volume). Creates missing folders.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let written = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    })();
    let result = written.and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// `profiles/<name>.toml` for a new rule, from the executable's file name.
fn suggested_profile_path(exe: &str) -> String {
    let name = file_name(exe.trim());
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let stem: String = stem
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let stem = if stem.is_empty() { "profile" } else { &stem };
    format!("profiles/{stem}.toml")
}

// ---------------------------------------------------------------------------
// Rule checks

/// Problems per rule (same order as `rules.apps`), from the core checks plus
/// rules that an earlier rule hides.
fn rule_issues(rules: &AppProfiles) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = rules
        .apps
        .iter()
        .map(|r| {
            let one = AppProfiles {
                default: None,
                apps: vec![r.clone()],
            };
            match one.validate() {
                Err(app_profiles::AppProfilesError::Rule { problem, .. }) => {
                    vec![problem.to_owned()]
                }
                Err(e) => vec![e.to_string()],
                Ok(()) => Vec::new(),
            }
        })
        .collect();
    for (i, j) in rules.shadowed() {
        out[i].push(format!(
            "same application as rule {}; this rule never matches",
            j + 1
        ));
    }
    out
}

/// Problem with `default`, if any.
fn default_issue(rules: &AppProfiles) -> Option<String> {
    let only_default = AppProfiles {
        default: rules.default.clone(),
        apps: Vec::new(),
    };
    only_default.validate().err().map(|e| e.to_string())
}

/// Non-blocking hint for an `exe` pattern that is unlikely to match.
fn exe_hint(exe: &str) -> Option<&'static str> {
    let name = file_name(exe.trim());
    (!name.is_empty() && !name.contains('.'))
        .then_some("Windows reports the full file name, e.g. cs2.exe")
}

// ---------------------------------------------------------------------------
// Profile form

fn registry_device(pid: u16) -> Option<&'static Device> {
    Registry::embedded()
        .lookup(RAZER_VID, pid)
        .map(|m| m.device())
}

fn limits(pid: u16) -> DeviceLimits {
    limits_for(registry_device(pid))
}

/// The settings of one profile file as the editor shows them. Sections the
/// editor does not edit (button assignments) are kept as read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileForm {
    /// `[device] pid` as typed (four hex digits).
    pid: String,
    dpi_on: bool,
    stages: Vec<u16>,
    /// 0-based position of the active stage.
    active: usize,
    polling_on: bool,
    hz: u32,
    sleep_on: bool,
    sleep_seconds: u16,
    /// Buttons and HyperShift buttons from the file, written back unchanged.
    kept: Profile,
}

impl ProfileForm {
    fn new(pid: Option<u16>) -> Self {
        Self {
            pid: pid.map(|p| Pid(p).to_string()).unwrap_or_default(),
            dpi_on: false,
            stages: vec![800, 1600],
            active: 0,
            polling_on: false,
            hz: 1000,
            sleep_on: false,
            sleep_seconds: 300,
            kept: Profile::default(),
        }
    }

    fn from_profile(p: &Profile) -> Self {
        let mut f = Self::new(p.device.as_ref().map(|d| d.pid.0));
        if let Some(d) = &p.dpi {
            f.dpi_on = true;
            f.stages.clone_from(&d.stages);
            f.active = usize::from(d.active_stage).saturating_sub(1);
        }
        if let Some(pl) = &p.polling {
            f.polling_on = true;
            f.hz = pl.hz;
        }
        if let Some(pw) = &p.power {
            f.sleep_on = true;
            f.sleep_seconds = pw.sleep_seconds;
        }
        f.kept = Profile {
            buttons: p.buttons.clone(),
            hypershift: p.hypershift.clone(),
            ..Profile::default()
        };
        f
    }

    fn pid(&self) -> Result<u16, String> {
        if self.pid.trim().is_empty() {
            return Err("needed: the agent applies the profile to devices with this PID".into());
        }
        Pid::try_from(self.pid.trim().to_owned()).map(|p| p.0)
    }

    fn buttons_kept(&self) -> usize {
        self.kept.all_buttons().count()
    }

    /// The profile to write, checked by `Profile::validate` against the
    /// registry limits of its PID. All problems at once.
    fn to_profile(&self) -> Result<Profile, Vec<String>> {
        let pid = self.pid().map_err(|e| vec![format!("device.pid: {e}")])?;
        let mut p = self.kept.clone();
        p.device = Some(DeviceSection { pid: Pid(pid) });
        p.dpi = self.dpi_on.then(|| DpiSection {
            stages: self.stages.clone(),
            active_stage: u8::try_from(self.active + 1).unwrap_or(u8::MAX),
        });
        p.polling = self.polling_on.then_some(PollingSection { hz: self.hz });
        p.power = self.sleep_on.then_some(PowerSection {
            sleep_seconds: self.sleep_seconds,
        });
        let mut errs = Vec::new();
        if p.dpi.is_none() && p.polling.is_none() && p.power.is_none() && self.buttons_kept() == 0 {
            errs.push("the profile sets nothing; tick at least one setting".to_owned());
        }
        if let Err(v) = p.validate(&limits(pid)) {
            errs.extend(v.0.iter().map(ToString::to_string));
        }
        if errs.is_empty() { Ok(p) } else { Err(errs) }
    }
}

// ---------------------------------------------------------------------------
// View state

/// Which profile path the settings editor belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Rule(usize),
    Default,
}

struct Editor {
    target: Target,
    form: ProfileForm,
    /// The form as loaded, to see whether the file needs writing.
    orig: ProfileForm,
    /// Resolved file the form was loaded from.
    loaded_from: PathBuf,
    /// The file does not exist yet; Save creates it.
    new_file: bool,
    /// The file could not be read or parsed; Save refuses until "Start new".
    blocked: Option<String>,
}

impl Editor {
    fn dirty(&self, path: &Path) -> bool {
        self.form != self.orig || self.new_file || path != self.loaded_from
    }
}

enum RowAction {
    Edit(Target),
    Up(usize),
    Down(usize),
    Remove(usize),
    Add,
}

enum Msg {
    Probed(AgentState),
    Reloaded(AgentState),
}

/// The "Per-app profiles" window. [`Self::button`] opens it, [`Self::window`]
/// draws it.
pub struct AppProfilesView {
    open: bool,
    /// The rules were read since the window opened.
    loaded: bool,
    /// The one agent call in flight.
    rx: Option<Receiver<Msg>>,
    agent: AgentState,
    file: Option<PathBuf>,
    /// The rules file is unreadable or invalid; editing is off until "Start empty".
    load_error: Option<String>,
    note: Option<String>,
    rules: AppProfiles,
    /// The rules as last read or saved.
    saved: AppProfiles,
    editor: Option<Editor>,
    /// Result of the last save (plus the agent's answer).
    status: Option<Result<String, String>>,
}

impl Default for AppProfilesView {
    fn default() -> Self {
        Self {
            open: false,
            loaded: false,
            rx: None,
            agent: AgentState::Asking,
            file: None,
            load_error: None,
            note: None,
            rules: AppProfiles::default(),
            saved: AppProfiles::default(),
            editor: None,
            status: None,
        }
    }
}

impl AppProfilesView {
    /// Top-bar button that opens and closes the window.
    pub fn button(&mut self, ui: &mut Ui) {
        if ui
            .selectable_label(self.open, "Per-app profiles")
            .on_hover_text("Which profile the agent applies while an application is in front")
            .clicked()
        {
            self.open = !self.open;
        }
    }

    /// Draw the window if it is open. `pid_hint`: PID of the device selected in
    /// the main window, used to prefill new profiles.
    pub fn window(&mut self, ctx: &egui::Context, pid_hint: Option<u16>) {
        self.drain();
        if !self.open {
            // Read again on the next open unless there are unsaved edits.
            if !self.dirty() {
                self.loaded = false;
            }
            return;
        }
        if !self.loaded {
            self.loaded = true;
            self.reload_from_disk(ctx);
        }
        let mut open = self.open;
        egui::Window::new("Per-app profiles")
            .open(&mut open)
            .default_width(700.0)
            .default_height(560.0)
            .vscroll(true)
            .show(ctx, |ui| self.contents(ui, pid_hint));
        self.open = open;
    }

    fn dirty(&self) -> bool {
        self.rules != self.saved
            || self
                .editor
                .as_ref()
                .is_some_and(|e| self.profile_path(e.target).is_some_and(|p| e.dirty(&p)))
    }

    fn drain(&mut self) {
        let msg = match self.rx.as_ref().map(Receiver::try_recv) {
            Some(Ok(m)) => m,
            Some(Err(TryRecvError::Disconnected)) => {
                self.rx = None;
                return;
            }
            Some(Err(TryRecvError::Empty)) | None => return,
        };
        self.rx = None;
        match msg {
            Msg::Probed(a) => {
                self.file = rules_file(&a, std::env::var("APPDATA").ok().as_deref());
                self.agent = a;
                self.load_rules();
            }
            Msg::Reloaded(a) => {
                let saved = match &self.status {
                    Some(Ok(s)) => s.clone(),
                    _ => String::new(),
                };
                self.status = Some(match reload_note(&a) {
                    Ok(n) => Ok(format!("{saved} {n}")),
                    Err(e) => Err(format!("{saved} {e}")),
                });
                self.agent = a;
            }
        }
    }

    /// Ask the agent in the background; `wrap` tags the answer.
    fn ask(&mut self, ctx: &egui::Context, req: Request, wrap: fn(AgentState) -> Msg) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("app-profiles-ipc".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let _ = tx.send(wrap(ask_agent(req)));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => {
                self.rx = Some(rx);
                self.agent = AgentState::Asking;
            }
            Err(e) => self.agent = AgentState::Error(format!("cannot start a thread: {e}")),
        }
    }

    /// Discard edits, ask the agent which file it reads, then read that file.
    fn reload_from_disk(&mut self, ctx: &egui::Context) {
        self.editor = None;
        self.status = None;
        self.ask(ctx, Request::AppProfiles, Msg::Probed);
    }

    fn load_rules(&mut self) {
        self.load_error = None;
        self.note = None;
        self.rules = AppProfiles::default();
        let Some(file) = &self.file else {
            self.load_error =
                Some("No rules file location: the APPDATA variable is not set.".into());
            self.saved = self.rules.clone();
            return;
        };
        match fs::read_to_string(file) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.note = Some("The file does not exist yet; Save creates it.".into());
            }
            Err(e) => self.load_error = Some(format!("cannot read {}: {e}", file.display())),
            Ok(text) => match AppProfiles::from_toml(&text) {
                Ok(r) => self.rules = r,
                Err(e) => self.load_error = Some(format!("{}: {e}", file.display())),
            },
        }
        self.saved = self.rules.clone();
    }

    /// The profile path (as written in the rules) of `t`.
    fn profile_ref(&self, t: Target) -> Option<&str> {
        match t {
            Target::Rule(i) => self.rules.apps.get(i).map(|r| r.profile.as_str()),
            Target::Default => self.rules.default.as_deref(),
        }
    }

    /// `profile_ref` resolved against the rules file.
    fn profile_path(&self, t: Target) -> Option<PathBuf> {
        let rel = self.profile_ref(t)?.trim();
        let file = self.file.as_deref()?;
        (!rel.is_empty()).then(|| resolve_profile_path(file, rel))
    }

    fn open_editor(&mut self, t: Target, pid_hint: Option<u16>) {
        if let Some(e) = &self.editor {
            if e.target == t {
                if !self.profile_path(t).is_some_and(|p| e.dirty(&p)) {
                    self.editor = None;
                }
                return;
            }
            if self.profile_path(e.target).is_some_and(|p| e.dirty(&p)) {
                self.status = Some(Err("Save or close the open profile settings first.".into()));
                return;
            }
        }
        let Some(path) = self.profile_path(t) else {
            self.status = Some(Err("Enter a profile file first.".into()));
            return;
        };
        let fresh = ProfileForm::new(pid_hint);
        let (form, new_file, blocked) = match fs::read_to_string(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (fresh, true, None),
            Err(e) => (
                fresh,
                false,
                Some(format!("cannot read {}: {e}", path.display())),
            ),
            Ok(text) => match Profile::from_toml(&text) {
                Ok(p) => (ProfileForm::from_profile(&p), false, None),
                Err(e) => (fresh, false, Some(format!("{}: {e}", path.display()))),
            },
        };
        self.status = None;
        self.editor = Some(Editor {
            target: t,
            orig: form.clone(),
            form,
            loaded_from: path,
            new_file,
            blocked,
        });
    }

    fn apply(&mut self, a: RowAction, pid_hint: Option<u16>) {
        let fix = |ed: &mut Option<Editor>, map: &dyn Fn(usize) -> Option<usize>| {
            if let Some(e) = ed
                && let Target::Rule(i) = e.target
            {
                match map(i) {
                    Some(j) => e.target = Target::Rule(j),
                    None => *ed = None,
                }
            }
        };
        match a {
            RowAction::Edit(t) => self.open_editor(t, pid_hint),
            RowAction::Add => self.rules.apps.push(AppRule {
                exe: String::new(),
                profile: String::new(),
            }),
            RowAction::Remove(r) => {
                self.rules.apps.remove(r);
                fix(&mut self.editor, &|i| match i.cmp(&r) {
                    std::cmp::Ordering::Less => Some(i),
                    std::cmp::Ordering::Equal => None,
                    std::cmp::Ordering::Greater => Some(i - 1),
                });
            }
            RowAction::Up(r) | RowAction::Down(r) => {
                let s = if matches!(a, RowAction::Up(_)) {
                    r - 1
                } else {
                    r + 1
                };
                self.rules.apps.swap(r, s);
                fix(&mut self.editor, &|i| {
                    Some(if i == r {
                        s
                    } else if i == s {
                        r
                    } else {
                        i
                    })
                });
            }
        }
    }

    /// Everything that blocks Save, as one line (`None`: Save is possible).
    fn save_blocker(&self, issues: &[Vec<String>], default: Option<&String>) -> Option<String> {
        if self.file.is_none() || self.load_error.is_some() {
            return Some("The rules file could not be read.".into());
        }
        if issues.iter().any(|v| !v.is_empty()) || default.is_some() {
            return Some("Fix the marked rules to save.".into());
        }
        if let Some(e) = &self.editor {
            if e.blocked.is_some() {
                return Some(
                    "The profile file could not be read; start a new one or close it.".into(),
                );
            }
            if e.form.to_profile().is_err() {
                return Some("Fix the marked profile settings to save.".into());
            }
        }
        None
    }

    /// Write the open profile (if changed) and the rules, then ask the agent to reload.
    fn save(&mut self, ctx: &egui::Context) {
        match self.write_files() {
            Ok(msg) => {
                self.status = Some(Ok(msg));
                self.ask(ctx, Request::ReloadAppProfiles, Msg::Reloaded);
            }
            Err(e) => self.status = Some(Err(e)),
        }
    }

    fn write_files(&mut self) -> Result<String, String> {
        let file = self.file.clone().ok_or("no rules file location")?;
        let mut written = Vec::new();
        if let Some(t) = self.editor.as_ref().map(|e| e.target) {
            let path = self.profile_path(t).ok_or("the open profile has no file")?;
            let ed = self.editor.as_mut().ok_or("no editor")?;
            if ed.dirty(&path) {
                let profile = ed.form.to_profile().map_err(|e| e.join("; "))?;
                let text = profile.to_toml().map_err(|e| e.to_string())?;
                write_atomic(&path, &text)
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                ed.orig = ed.form.clone();
                ed.loaded_from = path.clone();
                ed.new_file = false;
                written.push(path.display().to_string());
            }
        }
        let text = self.rules.to_toml().map_err(|e| e.to_string())?;
        write_atomic(&file, &text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
        self.saved = self.rules.clone();
        self.note = None;
        written.push(file.display().to_string());
        let missing: Vec<String> = self
            .rules
            .apps
            .iter()
            .map(|r| r.profile.as_str())
            .chain(self.rules.default.as_deref())
            .filter(|p| !resolve_profile_path(&file, p).is_file())
            .map(str::to_owned)
            .collect();
        let mut msg = format!("Saved {}.", written.join(" and "));
        if !missing.is_empty() {
            msg.push_str(&format!(
                " Profile file(s) not found: {}.",
                missing.join(", ")
            ));
        }
        Ok(msg)
    }

    // -----------------------------------------------------------------------
    // Drawing

    fn contents(&mut self, ui: &mut Ui, pid_hint: Option<u16>) {
        let err = ui.visuals().error_fg_color;
        let warn = ui.visuals().warn_fg_color;
        let idle = self.rx.is_none();

        ui.horizontal_wrapped(|ui| {
            ui.label("Rules file:");
            ui.monospace(
                self.file
                    .as_ref()
                    .map_or("(unknown)".into(), |f| f.display().to_string()),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(agent_line(&self.agent));
            if ui
                .add_enabled(idle, egui::Button::new("Reload from disk"))
                .on_hover_text("Discard unsaved edits and read the files again")
                .clicked()
            {
                self.reload_from_disk(ui.ctx());
            }
        });
        if let Some(n) = &self.note {
            ui.label(n);
        }
        if let Some(e) = self.load_error.clone() {
            ui.colored_label(err, e);
            if self.file.is_some()
                && ui
                    .button("Start with an empty list (replaces the file on Save)")
                    .clicked()
            {
                self.load_error = None;
            }
            return;
        }
        if self.file.is_none() {
            // Still asking the agent which file it reads.
            ui.spinner();
            return;
        }

        ui.separator();
        ui.label(RichText::new("Rules (the first match wins)").strong());
        let issues = rule_issues(&self.rules);
        let default_problem = default_issue(&self.rules);
        let mut action = None;
        let n = self.rules.apps.len();
        let editing = self.editor.as_ref().map(|e| e.target);
        for (i, rule) in self.rules.apps.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}.", i + 1));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut rule.exe)
                        .hint_text("cs2.exe")
                        .desired_width(170.0),
                );
                if r.lost_focus() && rule.profile.trim().is_empty() && !rule.exe.trim().is_empty() {
                    rule.profile = suggested_profile_path(&rule.exe);
                }
                ui.label("->");
                ui.add(
                    egui::TextEdit::singleline(&mut rule.profile)
                        .hint_text("profiles/cs2.toml")
                        .desired_width(200.0),
                );
                if ui
                    .selectable_label(editing == Some(Target::Rule(i)), "Settings")
                    .on_hover_text("Edit the settings in this profile file")
                    .clicked()
                {
                    action = Some(RowAction::Edit(Target::Rule(i)));
                }
                if ui.add_enabled(i > 0, egui::Button::new("Up")).clicked() {
                    action = Some(RowAction::Up(i));
                }
                if ui
                    .add_enabled(i + 1 < n, egui::Button::new("Down"))
                    .clicked()
                {
                    action = Some(RowAction::Down(i));
                }
                if ui.button("Remove").clicked() {
                    action = Some(RowAction::Remove(i));
                }
            });
            for m in &issues[i] {
                ui.colored_label(err, format!("    rule {}: {m}", i + 1));
            }
            if let Some(h) = exe_hint(&rule.exe) {
                ui.colored_label(warn, format!("    rule {}: {h}", i + 1));
            }
        }
        if n == 0 {
            ui.label("No rules yet.");
        }
        if ui.button("Add rule").clicked() {
            action = Some(RowAction::Add);
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut on = self.rules.default.is_some();
            if ui
                .checkbox(&mut on, "Default profile (no rule matches):")
                .changed()
            {
                self.rules.default = on.then(String::new);
                if !on && editing == Some(Target::Default) {
                    self.editor = None;
                }
            }
            if let Some(d) = &mut self.rules.default {
                ui.add(
                    egui::TextEdit::singleline(d)
                        .hint_text("profiles/desktop.toml")
                        .desired_width(200.0),
                );
                if ui
                    .selectable_label(editing == Some(Target::Default), "Settings")
                    .clicked()
                {
                    action = Some(RowAction::Edit(Target::Default));
                }
            }
        });
        if let Some(e) = &default_problem {
            ui.colored_label(err, format!("    {e}"));
        }
        if self.rules.default.is_none() {
            ui.weak("Without a default, leaving a matched app restores the values from before.");
        }
        if let Some(a) = action {
            self.apply(a, pid_hint);
        }

        if let Some(t) = self.editor.as_ref().map(|e| e.target) {
            ui.separator();
            let label = match t {
                Target::Rule(i) => format!(
                    "Profile settings for rule {} ({})",
                    i + 1,
                    self.rules.apps.get(i).map_or("", |r| r.exe.as_str())
                ),
                Target::Default => "Default profile settings".into(),
            };
            let path = self.profile_path(t);
            let mut close = false;
            if let Some(ed) = &mut self.editor {
                close = editor_ui(ui, ed, &label, path.as_deref(), pid_hint);
            }
            if close {
                self.editor = None;
            }
        }

        ui.separator();
        let blocker = self.save_blocker(&issues, default_problem.as_ref());
        let dirty = self.dirty();
        ui.horizontal(|ui| {
            let can = idle && dirty && blocker.is_none();
            if ui
                .add_enabled(can, egui::Button::new(RichText::new("Save").strong()))
                .on_hover_text("Write the files (atomically) and ask the agent to reload")
                .clicked()
            {
                self.save(ui.ctx());
            }
            if let Some(b) = &blocker {
                ui.colored_label(warn, b);
            } else if !dirty {
                ui.weak("No unsaved changes.");
            }
            if !idle {
                ui.spinner();
            }
        });
        match &self.status {
            Some(Ok(s)) => {
                ui.label(s);
            }
            Some(Err(e)) => {
                ui.colored_label(err, e);
            }
            None => {}
        }
        ui.weak(
            "Saving writes files only. The agent switches profiles when the app in front \
             changes. DPI stages are switched in volatile memory; polling, sleep and buttons \
             are written to the mouse on each switch.",
        );
    }
}

/// The settings editor for one profile file. Returns `true` when "Close" was clicked.
fn editor_ui(
    ui: &mut Ui,
    ed: &mut Editor,
    label: &str,
    path: Option<&Path>,
    pid_hint: Option<u16>,
) -> bool {
    let err = ui.visuals().error_fg_color;
    let warn = ui.visuals().warn_fg_color;
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).strong());
        if ui
            .button("Close")
            .on_hover_text("Close without saving this profile")
            .clicked()
        {
            close = true;
        }
    });
    match path {
        Some(p) => {
            let mut s = format!("File: {}", p.display());
            if ed.new_file {
                s.push_str(" (new; Save creates it)");
            } else if p != ed.loaded_from {
                s.push_str(" (Save writes the settings to this new path)");
            }
            ui.label(s);
        }
        None => {
            ui.colored_label(err, "Enter a profile file for this rule.");
        }
    }
    if let Some(b) = ed.blocked.clone() {
        ui.colored_label(err, b);
        if ui
            .button("Start a new profile here (replaces the file on Save)")
            .clicked()
        {
            ed.blocked = None;
            ed.form = ProfileForm::new(pid_hint);
            ed.new_file = true;
        }
        return close;
    }

    let f = &mut ed.form;
    let pid = f.pid();
    let lim = pid
        .as_ref()
        .map_or_else(|_| DeviceLimits::default(), |&p| limits(p));
    ui.horizontal(|ui| {
        ui.label("Device PID");
        ui.add(
            egui::TextEdit::singleline(&mut f.pid)
                .hint_text("00B8")
                .desired_width(60.0),
        );
        match &pid {
            Ok(p) => match registry_device(*p) {
                Some(d) => {
                    ui.label(&d.name);
                }
                None => {
                    ui.colored_label(warn, "not in the device registry (no DPI limits known)");
                }
            },
            Err(_) => {
                if let Some(h) = pid_hint
                    && ui
                        .button(format!("Use selected device ({})", Pid(h)))
                        .clicked()
                {
                    f.pid = Pid(h).to_string();
                }
            }
        }
    });

    ui.checkbox(&mut f.dpi_on, "DPI stages (volatile on switch)");
    if f.dpi_on {
        let mut remove = None;
        let step = f64::from(lim.dpi_step.max(1));
        for (i, v) in f.stages.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.active, i, format!("Stage {}", i + 1))
                    .on_hover_text("Active stage");
                ui.add(
                    egui::DragValue::new(v)
                        .range(lim.dpi_min..=lim.dpi_max)
                        .speed(step / 4.0)
                        .suffix(" DPI"),
                );
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            f.stages.remove(i);
            if f.active >= f.stages.len() {
                f.active = f.stages.len().saturating_sub(1);
            }
        }
        if f.stages.len() < lim.max_dpi_stages && ui.button("Add stage").clicked() {
            let next = f.stages.last().copied().unwrap_or(800);
            f.stages.push(next);
        }
    }

    ui.horizontal(|ui| {
        ui.checkbox(&mut f.polling_on, "Polling rate");
        if f.polling_on {
            egui::ComboBox::from_id_salt("app_profile_polling")
                .selected_text(format!("{} Hz", f.hz))
                .show_ui(ui, |ui| {
                    for hz in POLLING_RATES {
                        ui.selectable_value(&mut f.hz, hz, format!("{hz} Hz"));
                    }
                });
        }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.sleep_on, "Sleep timer");
        if f.sleep_on {
            ui.add(
                egui::DragValue::new(&mut f.sleep_seconds)
                    .range(SLEEP_RANGE)
                    .suffix(" s"),
            );
        }
    });
    let kept = f.buttons_kept();
    if kept > 0 {
        ui.label(format!(
            "{kept} button assignment(s) in this file are kept unchanged (edit them in the TOML file)."
        ));
    }
    if let Err(errs) = f.to_profile() {
        for e in errs {
            ui.colored_label(err, format!("    {e}"));
        }
    }
    close
}

#[cfg(test)]
mod tests;
