//! CLI dispatch. See README.md.
//!
//! Reads are always safe. Writes are dry runs unless `--apply` is given; they show
//! the current value first, write, read back, and print how to restore.

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use opensynapse_device_registry::{Device, Match, Protocol, RAZER_VID, Registry};
use opensynapse_hid::{DEFAULT_INTERFACE, DeviceSession, HidEntry, HidError, Timing, Transport};
use opensynapse_protocol::buttons::{self, ButtonAssignment, ButtonReply, Known, Mode};
use opensynapse_protocol::{Request, Status, device, dpi, hid_usage, polling, power};
use std::path::PathBuf;

mod agent;
mod hid_probe;
mod install;
mod lifecycle;
mod profile;
mod settings;

pub use agent::AgentAction;
pub use hid_probe::HidAction;
pub use settings::{SettingKey, SettingsAction, hint_for as wake_hint_for};

#[derive(Parser)]
#[command(
    name = "opensynapse",
    version,
    about = "Local-first Razer device configuration"
)]
pub struct Cli {
    /// USB product ID of the device to use (hex, e.g. 00B8). Needed when several are connected.
    #[arg(long, global = true, value_parser = parse_hex_u16)]
    pub pid: Option<u16>,
    /// Log every report sent and received.
    #[arg(short, long, global = true)]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List connected Razer devices.
    List,
    /// Firmware, serial, mode and known capabilities.
    Info,
    /// Battery level and charging state.
    Battery,
    /// Read or set the active DPI.
    Dpi {
        #[command(subcommand)]
        action: DpiAction,
    },
    /// Show the DPI stages (default) or set the active stage.
    DpiStages {
        #[command(subcommand)]
        action: Option<DpiStagesAction>,
    },
    /// Read or set the polling rate.
    Polling {
        #[command(subcommand)]
        action: PollingAction,
        /// Command variant: `period` (00/05) or `hyper` (00/40). Default: try period, then hyper.
        #[arg(long, global = true)]
        variant: Option<PollingVariant>,
        /// Profile byte for the hyper variant (meaning unconfirmed).
        #[arg(long, global = true, default_value_t = 0)]
        profile: u8,
    },
    /// Read or set the idle time before sleep (raw device value).
    Sleep {
        #[command(subcommand)]
        action: SleepAction,
    },
    /// Read or set an onboard button assignment.
    Buttons {
        #[command(subcommand)]
        action: ButtonsAction,
    },
    /// Export the device settings to a TOML profile, or apply one.
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Talk to the background agent (opensynapse-agent).
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// HID diagnostics: list collections, listen for input reports (read-only).
    Hid {
        #[command(subcommand)]
        action: HidAction,
    },
    /// User settings of the CLI and GUI (`settings.toml`).
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
    /// Copy CLI, agent and GUI from this build to the per-user install folder
    /// (%LOCALAPPDATA%\Programs\OpenSynapse; Linux ~/.local/bin), stopping and restarting a running
    /// agent, and point the login autostart at the installed agent.
    Install {
        /// Install folder instead of the default.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Do not turn autostart on. An entry for this build is still moved to
        /// the installed agent; any other entry is left alone.
        #[arg(long)]
        no_autostart: bool,
    },
    /// Stop the installed agent, remove its autostart entry and delete the
    /// installed binaries. Settings in %APPDATA%\OpenSynapse (Linux ~/.config/OpenSynapse) are kept.
    Uninstall {
        /// Install folder instead of the default.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Linux: print the udev rule that lets the logged-in user open Razer
    /// hidraw nodes without root (stdout), and how to install it (stderr).
    /// Only prints; never writes to /etc.
    UdevRule,
}

#[derive(Subcommand)]
pub enum ProfileAction {
    /// Read DPI stages (persistent), polling, sleep and button assignments (onboard
    /// profile 1, normal and HyperShift mode) and print them as a profile. A
    /// HyperShift entry is only written when it differs from the normal mapping.
    Export {
        /// Button IDs to read, e.g. `1-5` or `1,2,4`.
        #[arg(long, value_parser = profile::parse_button_list, default_value = "1-5")]
        buttons: profile::ButtonList,
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Apply a profile. Without --apply this only shows what would be sent. With
    /// --apply, the current values are saved to `<name>.restore.toml` first.
    Apply {
        /// Profile TOML file.
        file: PathBuf,
        #[command(flatten)]
        write: WriteOpts,
        /// Write the DPI stages to volatile storage (byte 0) instead of persistent (1).
        #[arg(long)]
        volatile: bool,
        /// Also write settings that already have the profile's value (normally skipped).
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
pub enum DpiAction {
    Get,
    /// Set DPI. Without --apply this only shows what would be sent.
    Set {
        /// DPI for X (and Y unless --y is given).
        dpi: u16,
        #[arg(long)]
        y: Option<u16>,
        #[command(flatten)]
        write: WriteOpts,
        /// Store on the device (storage byte 1, survives power-off) instead of volatile (0).
        #[arg(long)]
        persistent: bool,
    },
}

#[derive(Subcommand)]
pub enum DpiStagesAction {
    /// Show the stages; `*` marks the active one.
    Get {
        /// Read the persistent copy (storage byte 1) instead of the volatile one (0).
        #[arg(long)]
        persistent: bool,
    },
    /// Make another stage active. Without --apply this only shows what would be sent.
    Set {
        /// Position of the stage in the list, 1 = first.
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..))]
        active: u8,
        /// Store on the device (storage byte 1, survives power-off) instead of volatile (0).
        #[arg(long)]
        persistent: bool,
        #[command(flatten)]
        write: WriteOpts,
    },
}

#[derive(Subcommand)]
pub enum PollingAction {
    Get,
    Set {
        /// Rate in Hz.
        hz: u32,
        #[command(flatten)]
        write: WriteOpts,
    },
}

#[derive(Subcommand)]
pub enum SleepAction {
    Get,
    Set {
        /// Idle time in seconds.
        value: u16,
        #[command(flatten)]
        write: WriteOpts,
    },
}

#[derive(Subcommand)]
pub enum ButtonsAction {
    /// Show what a button is assigned to.
    Get {
        /// Button ID on the device.
        button: u8,
        #[command(flatten)]
        slot: ButtonSlot,
    },
    /// Assign a button. Without --apply this only shows what would be sent.
    Set {
        /// Button ID on the device.
        button: u8,
        /// What the button should do.
        action: ButtonKind,
        /// mouse-button: target button (1 = left, required). double-click: target
        /// (default 1). key: key name (a-z, 0-9, enter, esc, backspace, tab, space, f1-f12).
        target: Option<String>,
        /// key only: modifiers held with the key, comma-separated (ctrl, shift, alt, win).
        #[arg(long, value_delimiter = ',', value_parser = parse_modifier)]
        mods: Vec<u8>,
        #[command(flatten)]
        slot: ButtonSlot,
        #[command(flatten)]
        write: WriteOpts,
    },
    /// Read 02/8E "Get Button Assignment List" and print one line per button
    /// (record layout inferred). `-v` also prints the raw reply.
    List {
        #[command(flatten)]
        slot: ButtonSlot,
        /// ID offset byte of the request.
        #[arg(long, default_value_t = 0)]
        offset: u8,
    },
}

#[derive(Args, Clone, Copy)]
pub struct ButtonSlot {
    /// Onboard profile.
    #[arg(long, default_value_t = 1)]
    profile: u8,
    /// Use HyperShift mode (byte 2 = 1) instead of normal mode (0).
    #[arg(long)]
    hypershift: bool,
}

impl ButtonSlot {
    pub(crate) fn mode(self) -> Mode {
        if self.hypershift {
            Mode::HyperShift
        } else {
            Mode::Normal
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ButtonKind {
    MouseButton,
    DoubleClick,
    Key,
    DpiUp,
    DpiDown,
    Off,
}

#[derive(Args, Clone, Copy)]
pub struct WriteOpts {
    /// Actually write to the device.
    #[arg(long)]
    apply: bool,
    /// Write even if Razer software is running or the value is outside the known range.
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum PollingVariant {
    Period,
    Hyper,
}

impl From<PollingVariant> for opensynapse_core::PollingVariant {
    fn from(v: PollingVariant) -> Self {
        match v {
            PollingVariant::Period => Self::Period,
            PollingVariant::Hyper => Self::Hyper,
        }
    }
}

impl From<opensynapse_core::PollingVariant> for PollingVariant {
    fn from(v: opensynapse_core::PollingVariant) -> Self {
        match v {
            opensynapse_core::PollingVariant::Period => Self::Period,
            opensynapse_core::PollingVariant::Hyper => Self::Hyper,
        }
    }
}

fn parse_modifier(s: &str) -> Result<u8, String> {
    hid_usage::modifier(s).ok_or_else(|| format!("unknown modifier `{s}` (ctrl, shift, alt, win)"))
}

fn parse_hex_u16(s: &str) -> Result<u16, String> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(t, 16).map_err(|e| format!("not a hex PID: {e}"))
}

/// `udev-rule`: the rule on stdout (so `> 70-opensynapse.rules` saves a clean
/// file), the install steps on stderr. Pure text, works on every OS.
fn udev_rule() -> Result<()> {
    use opensynapse_sys::udev;
    print!("{}", udev::UDEV_RULE);
    eprintln!(
        "\nSave the rule above as {} (e.g. `opensynapse udev-rule > {}`), then install it:\n{}",
        udev::RULE_FILE_NAME,
        udev::RULE_FILE_NAME,
        udev::install_hint()
    );
    Ok(())
}

pub fn run(cli: Cli) -> Result<()> {
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("debug")
            .with_writer(std::io::stderr)
            .init();
    }
    let verbose = cli.verbose;
    if let Command::Agent { action } = cli.command {
        return agent::run(action, cli.pid);
    }
    if let Command::Hid { action } = cli.command {
        return hid_probe::run(action, cli.pid);
    }
    if let Command::Settings { action } = cli.command {
        return settings::run(action);
    }
    if let Command::Install { dir, no_autostart } = cli.command {
        return install::install(dir, no_autostart);
    }
    if let Command::Uninstall { dir } = cli.command {
        return install::uninstall(dir);
    }
    if let Command::UdevRule = cli.command {
        return udev_rule();
    }
    let transport = Transport::new().context("cannot enumerate HID devices")?;
    if let Command::List = cli.command {
        return list(&transport);
    }

    let mut dev = Target::select(&transport, cli.pid)?;
    match cli.command {
        Command::List
        | Command::Agent { .. }
        | Command::Hid { .. }
        | Command::Settings { .. }
        | Command::Install { .. }
        | Command::Uninstall { .. }
        | Command::UdevRule => {
            unreachable!()
        }
        Command::Info => info(&mut dev),
        Command::Battery => battery(&mut dev),
        Command::Dpi { action } => match action {
            DpiAction::Get => {
                let d = dev.query(&dpi::get_dpi(dpi::Storage::Volatile), dpi::parse_dpi)?;
                println!("DPI: {d}");
                Ok(())
            }
            DpiAction::Set {
                dpi: x,
                y,
                write,
                persistent,
            } => dpi_set(&mut dev, x, y.unwrap_or(x), persistent, write),
        },
        Command::DpiStages { action } => match action {
            None => dpi_stages_get(&mut dev, false),
            Some(DpiStagesAction::Get { persistent }) => dpi_stages_get(&mut dev, persistent),
            Some(DpiStagesAction::Set {
                active,
                persistent,
                write,
            }) => dpi_stages_set(&mut dev, active, persistent, write),
        },
        Command::Polling {
            action,
            variant,
            profile,
        } => match action {
            PollingAction::Get => {
                let (hz, v) = polling_get(&mut dev, variant, profile)?;
                println!("Polling rate: {hz} Hz ({})", variant_name(v));
                Ok(())
            }
            PollingAction::Set { hz, write } => polling_set(&mut dev, variant, profile, hz, write),
        },
        Command::Sleep { action } => match action {
            SleepAction::Get => {
                let v = dev.query(&power::get_sleep_time(), power::parse_sleep_time)?;
                println!("Sleep timer: {v} s");
                Ok(())
            }
            SleepAction::Set { value, write } => {
                let old = dev.query(&power::get_sleep_time(), power::parse_sleep_time)?;
                dev.write(
                    write,
                    &format!("sleep timer {old} -> {value}"),
                    &format!("sleep set {old} --apply"),
                    &power::set_sleep_time(value),
                )?;
                if write.apply {
                    let now = dev.query(&power::get_sleep_time(), power::parse_sleep_time)?;
                    verify("sleep timer", now, value, "")?;
                }
                Ok(())
            }
        },
        Command::Buttons { action } => match action {
            ButtonsAction::Get { button, slot } => {
                let r = button_get(&mut dev, button, slot)?;
                println!("Button: {r}");
                if verbose {
                    println!("Reply byte 2: {} (meaning unknown)", r.reply_byte2);
                }
                Ok(())
            }
            ButtonsAction::Set {
                button,
                action,
                target,
                mods,
                slot,
                write,
            } => {
                let new = requested_assignment(button, action, target.as_deref(), &mods, slot)?;
                button_set(&mut dev, new, slot, write)
            }
            ButtonsAction::List { slot, offset } => buttons_list(&mut dev, slot, offset, verbose),
        },
        Command::Profile { action } => match action {
            ProfileAction::Export { buttons, out } => {
                profile::export(&mut dev, &buttons, out.as_deref())
            }
            ProfileAction::Apply {
                file,
                write,
                volatile,
                all,
            } => profile::apply(&mut dev, &file, write, volatile, all),
        },
    }
}

fn list(transport: &Transport) -> Result<()> {
    let reg = Registry::embedded();
    let mut entries = transport.entries().to_vec();
    entries.sort_by_key(|e| e.pid);
    entries.dedup_by_key(|e| e.pid);
    if entries.is_empty() {
        println!("No Razer devices found.");
        return Ok(());
    }
    let control = transport.control_entries(DEFAULT_INTERFACE);
    for e in &entries {
        let m = reg.lookup(e.vid, e.pid);
        let name = m.map_or("unknown Razer device", |m| m.device().name.as_str());
        let proto = m.map_or(Protocol::Unknown, |m| m.protocol());
        let ctl = if control.iter().any(|c| c.pid == e.pid) {
            "control: yes"
        } else {
            "control: no"
        };
        println!("{:04X}  {name}  (protocol {proto}, {ctl})", e.pid);
    }
    Ok(())
}

fn info(dev: &mut Target) -> Result<()> {
    println!("Device: {} ({:04X})", dev.name(), dev.entry.pid);
    let fw = dev.query(
        &device::get_firmware_version(),
        device::parse_firmware_version,
    )?;
    println!("Firmware: {fw}");
    print_opt(
        "Serial",
        dev.query(&device::get_serial(), device::parse_serial),
    );
    print_opt(
        "Device mode",
        dev.query(&device::get_device_mode(), device::parse_device_mode)
            .map(|(m, p)| format!("{m:?} (param {p})")),
    );
    if let Some(d) = dev.device {
        if let (Some(min), Some(max), Some(step)) = (d.dpi_min, d.dpi_max, d.dpi_step) {
            println!("DPI range: {min}-{max}, step {step}");
        }
        if !d.polling_rates.is_empty() {
            println!("Polling rates: {:?} Hz", d.polling_rates);
        }
        println!("Battery: {}", if d.battery { "yes" } else { "no" });
    }
    Ok(())
}

fn battery(dev: &mut Target) -> Result<()> {
    let level = dev.query(&power::get_battery_level(), power::parse_battery_level)?;
    println!("Battery: {}% (raw {})", level.percent(), level.0);
    print_opt(
        "Charging",
        dev.query(&power::get_charging_status(), power::parse_charging_status),
    );
    Ok(())
}

fn dpi_set(dev: &mut Target, x: u16, y: u16, persistent: bool, write: WriteOpts) -> Result<()> {
    if let Some(d) = dev.device {
        for v in [x, y] {
            if !d.dpi_valid(u32::from(v)) && !write.force {
                bail!(
                    "{v} DPI is outside {}-{} (step {}) for this device; use --force to override",
                    d.dpi_min.unwrap_or(0),
                    d.dpi_max.unwrap_or(0),
                    d.dpi_step.unwrap_or(1)
                );
            }
        }
    }
    let storage = storage(persistent);
    let old = dev.query(&dpi::get_dpi(storage), dpi::parse_dpi)?;
    let new = dpi::Dpi { x, y };
    dev.write(
        write,
        &format!("DPI {old} -> {new}"),
        &format!(
            "dpi set {} --y {}{} --apply",
            old.x,
            old.y,
            if persistent { " --persistent" } else { "" }
        ),
        &dpi::set_dpi(storage, new),
    )?;
    if write.apply {
        verify(
            "DPI",
            dev.query(&dpi::get_dpi(storage), dpi::parse_dpi)?,
            new,
            "",
        )?;
    }
    Ok(())
}

fn storage(persistent: bool) -> dpi::Storage {
    if persistent {
        dpi::Storage::Persistent
    } else {
        dpi::Storage::Volatile
    }
}

fn dpi_stages_get(dev: &mut Target, persistent: bool) -> Result<()> {
    let s = dev.query(
        &dpi::get_dpi_stages(storage(persistent)),
        dpi::parse_dpi_stages,
    )?;
    for (i, st) in s.stages.iter().enumerate() {
        let mark = if st.id == s.active { "*" } else { " " };
        // The CLI addresses stages by position; show it when it differs from the ID.
        let pos = if usize::from(st.id) == i + 1 {
            String::new()
        } else {
            format!(" (position {})", i + 1)
        };
        println!("{mark} stage {}: {}{pos}", st.id, st.dpi);
    }
    Ok(())
}

/// The stages with the stage at 1-based position `n` made active.
fn requested_stages(old: &dpi::DpiStages, n: u8) -> Result<dpi::DpiStages> {
    let count = old.stages.len();
    if n == 0 || usize::from(n) > count {
        bail!("stage {n} does not exist; the device has {count} stages (1-{count})");
    }
    Ok(old.with_active(usize::from(n) - 1)?)
}

/// 1-based position of the active stage, or the raw ID if no stage has it.
fn active_label(s: &dpi::DpiStages) -> String {
    match s.active_index() {
        Some(i) => (i + 1).to_string(),
        None => format!("? (ID {} matches no stage)", s.active),
    }
}

/// CLI arguments that make the active stage of `old` active again, or `None` if its
/// active ID matches no stage.
fn stages_restore(old: &dpi::DpiStages, persistent: bool) -> Option<String> {
    let i = old.active_index()?;
    Some(format!(
        "dpi-stages set --active {}{} --apply",
        i + 1,
        if persistent { " --persistent" } else { "" }
    ))
}

fn dpi_stages_set(dev: &mut Target, n: u8, persistent: bool, write: WriteOpts) -> Result<()> {
    let storage = storage(persistent);
    let old = dev.query(&dpi::get_dpi_stages(storage), dpi::parse_dpi_stages)?;
    let new = requested_stages(&old, n)?;
    let req = dpi::set_dpi_stages(storage, &new)?;
    let restore = stages_restore(&old, persistent);
    if restore.is_none() {
        println!(
            "Note: the current active stage ID {} matches no stage; write it down to restore it later.",
            old.active
        );
    }
    let before = dev.query(&dpi::get_dpi(dpi::Storage::Volatile), dpi::parse_dpi)?;
    println!("Current DPI (04/85): {before}");
    dev.write(
        write,
        &format!(
            "active DPI stage {} -> {}",
            active_label(&old),
            active_label(&new)
        ),
        restore.as_deref().unwrap_or(""),
        &req,
    )?;
    if write.apply {
        if restore.is_none() {
            println!(
                "To restore: not possible with the CLI (old active stage ID {}).",
                old.active
            );
        }
        let now = dev.query(&dpi::get_dpi_stages(storage), dpi::parse_dpi_stages)?;
        if now.stages != new.stages {
            bail!("DPI stage list changed after the write: {now:?}, expected {new:?}");
        }
        verify(
            "active DPI stage",
            active_label(&now),
            active_label(&new),
            "",
        )?;
        // Unknown whether setting the active stage also applies its DPI; show it.
        let after = dev.query(&dpi::get_dpi(dpi::Storage::Volatile), dpi::parse_dpi)?;
        let stage_dpi = new.stages[usize::from(n) - 1].dpi;
        let verdict = if after == before {
            "unchanged"
        } else if after == stage_dpi {
            "now equals the stage DPI"
        } else {
            "changed, but not to the stage DPI"
        };
        println!("Current DPI (04/85): {after} ({verdict}; stage {n} is {stage_dpi}).");
    }
    Ok(())
}

fn button_get(dev: &mut Target, button: u8, slot: ButtonSlot) -> Result<ButtonReply> {
    read_button(dev, slot.profile, button, slot.mode())
}

/// 02/8C for `mode`. The result carries the requested mode; reply byte 2 is kept
/// raw (`buttons::parse_button_assignment`).
pub(crate) fn read_button(
    dev: &mut Target,
    profile: u8,
    button: u8,
    mode: Mode,
) -> Result<ButtonReply> {
    dev.query(
        &buttons::get_button_assignment(profile, button, mode),
        |args| buttons::parse_button_assignment(args, mode),
    )
}

/// `buttons list`: send 02/8E once and print one line per record (plus the raw
/// reply with `verbose`).
fn buttons_list(dev: &mut Target, slot: ButtonSlot, offset: u8, verbose: bool) -> Result<()> {
    let mode = slot.mode();
    let l = dev.query(
        &buttons::get_button_assignment_list(slot.profile, mode, offset),
        buttons::parse_button_assignment_list,
    )?;
    if verbose {
        println!(
            "Request 02/8E: profile {}, mode {} ({mode}), offset {offset}",
            slot.profile, mode as u8
        );
        println!(
            "Reply: profile {}, mode {}, offset {}",
            l.profile, l.mode, l.id_offset
        );
        println!("Data: {}", hex_trimmed(&l.data));
    }
    let lines = list_lines(slot.profile, mode, &l);
    if lines.is_empty() {
        println!("No button records in the reply.");
    }
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// One `buttons get`-style line per 02/8E record, with the requested mode.
fn list_lines(profile: u8, mode: Mode, l: &buttons::ButtonAssignmentList) -> Vec<String> {
    l.records()
        .into_iter()
        .map(|(button, function, data)| {
            ButtonAssignment {
                profile,
                button,
                mode,
                function,
                data,
            }
            .to_string()
        })
        .collect()
}

/// Space-separated hex; trailing zero bytes are summarised.
fn hex_trimmed(data: &[u8]) -> String {
    let end = data.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let mut s: Vec<String> = data[..end].iter().map(|b| format!("{b:02x}")).collect();
    if end < data.len() {
        s.push(format!("(+{} zero bytes)", data.len() - end));
    }
    s.join(" ")
}

/// The assignment `buttons set` asks for.
fn requested_assignment(
    button: u8,
    kind: ButtonKind,
    target: Option<&str>,
    mods: &[u8],
    slot: ButtonSlot,
) -> Result<ButtonAssignment> {
    let (p, l) = (slot.profile, slot.mode());
    if !mods.is_empty() && kind != ButtonKind::Key {
        bail!("--mods only applies to `key`");
    }
    let number = |t: &str| -> Result<u8> {
        t.parse()
            .with_context(|| format!("`{t}` is not a button number"))
    };
    Ok(match kind {
        ButtonKind::MouseButton => {
            let Some(t) = target else {
                bail!("mouse-button needs a target button, e.g. `mouse-button 4`")
            };
            buttons::mouse_button(p, button, l, number(t)?)
        }
        ButtonKind::DoubleClick => {
            buttons::double_click(p, button, l, target.map_or(Ok(1), number)?)
        }
        ButtonKind::Key => {
            let Some(t) = target else {
                bail!("key needs a key name, e.g. `key a`")
            };
            let Some(usage) = hid_usage::usage(t) else {
                bail!("unknown key `{t}` (a-z, 0-9, enter, esc, backspace, tab, space, f1-f12)")
            };
            let modifiers = mods.iter().fold(0, |m, b| m | b);
            buttons::keyboard(p, button, l, modifiers, usage)
        }
        ButtonKind::DpiUp => {
            if let Some(t) = target {
                bail!("dpi-up takes no argument (got `{t}`)");
            }
            buttons::dpi_up(p, button, l)
        }
        ButtonKind::DpiDown => {
            if let Some(t) = target {
                bail!("dpi-down takes no argument (got `{t}`)");
            }
            buttons::dpi_down(p, button, l)
        }
        ButtonKind::Off => {
            if let Some(t) = target {
                bail!("off takes no argument (got `{t}`)");
            }
            buttons::off(p, button, l)
        }
    })
}

/// CLI arguments that recreate `old`, or `None` if the CLI cannot express it yet.
fn button_restore(old: &ButtonAssignment) -> Option<String> {
    let action = match old.known()? {
        Known::MouseButton(t) => format!("mouse-button {t}"),
        Known::DoubleClick(t) => format!("double-click {t}"),
        Known::Key { modifiers, usage } => {
            let key = hid_usage::name(usage)?;
            let mods = hid_usage::modifier_names(modifiers)?;
            if mods.is_empty() {
                format!("key {key}")
            } else {
                format!("key {key} --mods {}", mods.join(","))
            }
        }
        Known::DpiUp => "dpi-up".to_string(),
        Known::DpiDown => "dpi-down".to_string(),
        Known::Off => "off".to_string(),
    };
    let hs = if old.mode == Mode::HyperShift {
        " --hypershift"
    } else {
        ""
    };
    Some(format!(
        "buttons set {} {action} --profile {}{hs} --apply",
        old.button, old.profile
    ))
}

fn button_set(
    dev: &mut Target,
    new: ButtonAssignment,
    slot: ButtonSlot,
    write: WriteOpts,
) -> Result<()> {
    let button = new.button;
    let req = buttons::set_button_assignment(&new)?;
    let old_reply = button_get(dev, button, slot)?;
    let old = &old_reply.assignment;
    let restore = button_restore(old);
    if restore.is_none() {
        println!(
            "Note: the current assignment (function {}, data {:?}) cannot be expressed by the \
             CLI yet; write these raw values down to restore it later.",
            old.function, old.data
        );
    }
    dev.write(
        write,
        &format!("button {old_reply} -> {new}"),
        restore.as_deref().unwrap_or(""),
        &req,
    )?;
    if write.apply {
        if restore.is_none() {
            println!(
                "To restore: not possible with the CLI yet (old function {}, data {:?}).",
                old.function, old.data
            );
        }
        verify("button", button_get(dev, button, slot)?.assignment, new, "")?;
    }
    Ok(())
}

fn polling_get(
    dev: &mut Target,
    variant: Option<PollingVariant>,
    profile: u8,
) -> Result<(u32, PollingVariant)> {
    let period =
        |d: &mut Target| d.query(&polling::get_polling_rate(), polling::parse_polling_rate);
    let hyper = |d: &mut Target| {
        d.query(
            &polling::get_hyper_polling(profile),
            polling::parse_hyper_polling,
        )
    };
    match variant {
        Some(PollingVariant::Period) => Ok((period(dev)?, PollingVariant::Period)),
        Some(PollingVariant::Hyper) => Ok((hyper(dev)?, PollingVariant::Hyper)),
        None => match period(dev) {
            Ok(hz) => Ok((hz, PollingVariant::Period)),
            Err(e) if unsupported(&e) => Ok((hyper(dev)?, PollingVariant::Hyper)),
            Err(e) => Err(e),
        },
    }
}

fn polling_set(
    dev: &mut Target,
    variant: Option<PollingVariant>,
    profile: u8,
    hz: u32,
    write: WriteOpts,
) -> Result<()> {
    if let Some(d) = dev.device
        && !d.polling_rates.is_empty()
        && !d.polling_rates.contains(&hz)
        && !write.force
    {
        bail!(
            "{hz} Hz is not offered for this device ({:?}); use --force to override",
            d.polling_rates
        );
    }
    let (old, v) = polling_get(dev, variant, profile)?;
    let req = polling_request(v, profile, hz)?;
    dev.write(
        write,
        &format!("polling rate {old} Hz -> {hz} Hz ({})", variant_name(v)),
        &format!("polling set {old} --variant {} --apply", variant_name(v)),
        &req,
    )?;
    if write.apply {
        verify(
            "polling rate",
            polling_get(dev, Some(v), profile)?.0,
            hz,
            " Hz",
        )?;
    }
    Ok(())
}

/// The set request for `hz` in variant `v` (`profile` is the HyperPolling profile byte).
fn polling_request(v: PollingVariant, profile: u8, hz: u32) -> Result<Request> {
    Ok(match v {
        PollingVariant::Period => polling::set_polling_rate(hz)?,
        PollingVariant::Hyper => polling::set_hyper_polling(profile, hz)?,
    })
}

fn variant_name(v: PollingVariant) -> &'static str {
    match v {
        PollingVariant::Period => "period",
        PollingVariant::Hyper => "hyper",
    }
}

fn unsupported(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<HidError>(),
        Some(HidError::Status(
            Status::NotSupported | Status::UnknownCommand | Status::WrongDeviceMode
        ))
    )
}

/// Compare a read-back value; `unit` is appended to both values (e.g. " Hz").
fn verify<T: PartialEq + std::fmt::Display>(what: &str, now: T, want: T, unit: &str) -> Result<()> {
    if now == want {
        println!("Verified: {what} is now {now}{unit}.");
        Ok(())
    } else {
        bail!("{what} reads back as {now}{unit}, expected {want}{unit}")
    }
}

fn print_opt<T: std::fmt::Display>(label: &str, r: Result<T>) {
    match r {
        Ok(v) => println!("{label}: {v}"),
        Err(e) => println!("{label}: n/a ({e})"),
    }
}

/// The selected device: its control collection, registry entry and open session.
struct Target {
    entry: HidEntry,
    device: Option<&'static Device>,
    session: DeviceSession,
    conflict: Option<&'static str>,
}

impl Target {
    fn select(transport: &Transport, pid: Option<u16>) -> Result<Self> {
        let reg = Registry::embedded();
        let candidates: Vec<HidEntry> = transport
            .control_entries(DEFAULT_INTERFACE)
            .into_iter()
            .filter(|e| pid.is_none_or(|p| e.pid == p))
            .collect();
        let entry = match candidates.as_slice() {
            [] => match pid {
                Some(p) => bail!("no Razer control interface found for PID {p:04X}"),
                None => {
                    bail!("no Razer device with a control interface found (see `opensynapse list`)")
                }
            },
            [one] => one.clone(),
            many => {
                let pids: Vec<String> = many.iter().map(|e| format!("{:04X}", e.pid)).collect();
                bail!(
                    "several devices found ({}); choose one with --pid",
                    pids.join(", ")
                )
            }
        };
        let m = reg.lookup(RAZER_VID, entry.pid);
        let protocol = m.map_or(Protocol::Unknown, |m| m.protocol());
        if protocol != Protocol::V25 {
            bail!(
                "{:04X} speaks protocol {protocol}; only 2.5 is implemented",
                entry.pid
            );
        }
        let conflict = opensynapse_hid::conflicting_process();
        if let Some(name) = conflict {
            eprintln!("warning: {name} is running and may talk to the device at the same time");
        }
        let session = transport.open(&entry, Timing::default())?;
        Ok(Self {
            device: m.map(|m: Match<'static>| m.device()),
            entry,
            session,
            conflict,
        })
    }

    fn name(&self) -> &str {
        self.device.map_or("unknown device", |d| d.name.as_str())
    }

    fn query<T>(
        &mut self,
        req: &Request,
        parse: impl FnOnce(&[u8]) -> Result<T, opensynapse_protocol::ProtocolError>,
    ) -> Result<T> {
        let reply = self.session.execute(req)?;
        Ok(parse(reply.args())?)
    }

    /// Refuses to write while Razer software runs unless `--force`.
    fn guard(&self, opts: WriteOpts) -> Result<()> {
        if let Some(name) = self.conflict
            && !opts.force
        {
            bail!("{name} is running; close it or pass --force");
        }
        Ok(())
    }

    /// Dry run unless `--apply`; refuses while Razer software runs unless `--force`.
    fn write(&mut self, opts: WriteOpts, change: &str, restore: &str, req: &Request) -> Result<()> {
        if !opts.apply {
            println!("Dry run: would change {change}.");
            println!("Report: {:?}", req.encode(0));
            println!("Run again with --apply to write.");
            return Ok(());
        }
        self.guard(opts)?;
        self.session.execute(req)?;
        println!("Changed {change}.");
        // Empty: the caller explains why there is no restore command.
        if !restore.is_empty() {
            println!(
                "To restore: opensynapse --pid {:04X} {restore}",
                self.entry.pid
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn hex_pid() {
        assert_eq!(parse_hex_u16("00B8"), Ok(0x00B8));
        assert_eq!(parse_hex_u16("0x00b8"), Ok(0x00B8));
        assert!(parse_hex_u16("zz").is_err());
    }

    #[test]
    fn udev_rule_parses() {
        let cli = Cli::try_parse_from(["opensynapse", "udev-rule"]).unwrap();
        assert!(matches!(cli.command, Command::UdevRule));
        assert!(Cli::try_parse_from(["opensynapse", "udev-rule", "extra"]).is_err());
    }

    #[test]
    fn writes_need_apply() {
        let cli = Cli::try_parse_from(["opensynapse", "dpi", "set", "800"]).unwrap();
        let Command::Dpi {
            action: DpiAction::Set { write, .. },
        } = cli.command
        else {
            panic!("parsed wrong command")
        };
        assert!(!write.apply && !write.force);
        let cli = Cli::try_parse_from([
            "opensynapse",
            "--pid",
            "00B8",
            "polling",
            "set",
            "1000",
            "--apply",
        ]);
        assert!(cli.is_ok());
    }

    fn parse_stages(args: &[&str]) -> Result<Option<DpiStagesAction>> {
        let argv = ["opensynapse", "dpi-stages"]
            .into_iter()
            .chain(args.iter().copied());
        let Command::DpiStages { action } = Cli::try_parse_from(argv)?.command else {
            panic!("parsed wrong command")
        };
        Ok(action)
    }

    fn stages(active: u8, ids: &[u8]) -> dpi::DpiStages {
        dpi::DpiStages {
            active,
            stages: ids
                .iter()
                .map(|&id| dpi::DpiStage {
                    id,
                    dpi: dpi::Dpi::uniform(400 * u16::from(id)),
                })
                .collect(),
        }
    }

    #[test]
    fn dpi_stages_parsing() {
        // Backward compatible: no subcommand still reads.
        assert!(parse_stages(&[]).unwrap().is_none());
        assert!(matches!(
            parse_stages(&["get"]).unwrap(),
            Some(DpiStagesAction::Get { persistent: false })
        ));
        let Some(DpiStagesAction::Set {
            active,
            persistent,
            write,
        }) = parse_stages(&["set", "--active", "1"]).unwrap()
        else {
            panic!("parsed wrong action")
        };
        assert_eq!(active, 1);
        assert!(!persistent && !write.apply && !write.force);
        let Some(DpiStagesAction::Set {
            persistent, write, ..
        }) = parse_stages(&["set", "--active", "3", "--persistent", "--apply", "--force"]).unwrap()
        else {
            panic!("parsed wrong action")
        };
        assert!(persistent && write.apply && write.force);
        assert!(parse_stages(&["set"]).is_err());
        assert!(parse_stages(&["set", "--active", "0"]).is_err());
        assert!(parse_stages(&["set", "--active", "x"]).is_err());
    }

    #[test]
    fn dpi_stages_request() {
        let old = stages(5, &[1, 2, 3, 4, 5]);
        let new = requested_stages(&old, 1).unwrap();
        assert_eq!(new.active, 1);
        assert_eq!(new.stages, old.stages);
        assert_eq!(
            (active_label(&old), active_label(&new)),
            ("5".into(), "1".into())
        );
        assert!(requested_stages(&old, 0).is_err());
        assert!(requested_stages(&old, 6).is_err());
        // Positions, not IDs.
        let odd = stages(7, &[3, 7]);
        assert_eq!(requested_stages(&odd, 1).unwrap().active, 3);
        assert_eq!(active_label(&odd), "2");
    }

    #[test]
    fn dpi_stages_restore_command() {
        assert_eq!(
            stages_restore(&stages(5, &[1, 2, 3, 4, 5]), false).as_deref(),
            Some("dpi-stages set --active 5 --apply")
        );
        assert_eq!(
            stages_restore(&stages(7, &[3, 7]), true).as_deref(),
            Some("dpi-stages set --active 2 --persistent --apply")
        );
        assert_eq!(stages_restore(&stages(9, &[1, 2]), false), None);
        // The restore command parses back and selects the old stage again.
        let old = stages(4, &[1, 2, 3, 4, 5]);
        let cmd = stages_restore(&old, true).unwrap();
        let args: Vec<&str> = cmd.split(' ').skip(1).collect();
        let Some(DpiStagesAction::Set {
            active, persistent, ..
        }) = parse_stages(&args).unwrap()
        else {
            panic!("parsed wrong action")
        };
        assert!(persistent);
        assert_eq!(requested_stages(&old, active).unwrap(), old);
    }

    fn parse_buttons_set(args: &[&str]) -> Result<(ButtonAssignment, WriteOpts)> {
        let argv = ["opensynapse", "buttons", "set"]
            .into_iter()
            .chain(args.iter().copied());
        let cli = Cli::try_parse_from(argv)?;
        let Command::Buttons {
            action:
                ButtonsAction::Set {
                    button,
                    action,
                    target,
                    mods,
                    slot,
                    write,
                },
        } = cli.command
        else {
            panic!("parsed wrong command")
        };
        Ok((
            requested_assignment(button, action, target.as_deref(), &mods, slot)?,
            write,
        ))
    }

    #[test]
    fn buttons_set_parsing() {
        let (a, w) = parse_buttons_set(&["4", "mouse-button", "5"]).unwrap();
        assert_eq!(a, buttons::mouse_button(1, 4, Mode::Normal, 5));
        assert!(!w.apply);

        let (a, w) = parse_buttons_set(&[
            "5",
            "double-click",
            "--profile",
            "2",
            "--hypershift",
            "--apply",
        ])
        .unwrap();
        assert_eq!(a, buttons::double_click(2, 5, Mode::HyperShift, 1));
        assert!(w.apply);

        let (a, _) = parse_buttons_set(&["4", "key", "a"]).unwrap();
        assert_eq!(a, buttons::keyboard(1, 4, Mode::Normal, 0x00, 0x04));
        let (a, _) = parse_buttons_set(&["4", "key", "F5", "--mods", "ctrl,shift"]).unwrap();
        assert_eq!(a, buttons::keyboard(1, 4, Mode::Normal, 0x03, 0x3E));
        let (a, _) = parse_buttons_set(&["4", "dpi-down"]).unwrap();
        assert_eq!(a, buttons::dpi_down(1, 4, Mode::Normal));
        let (a, _) = parse_buttons_set(&["4", "off"]).unwrap();
        assert_eq!(a, buttons::off(1, 4, Mode::Normal));
        assert!(parse_buttons_set(&["4", "off", "2"]).is_err());
        assert_eq!(
            button_restore(&buttons::off(1, 4, Mode::Normal)).as_deref(),
            Some("buttons set 4 off --profile 1 --apply")
        );
        let (a, _) = parse_buttons_set(&["4", "dpi-up"]).unwrap();
        assert_eq!(a, buttons::dpi_up(1, 4, Mode::Normal));

        assert!(parse_buttons_set(&["4", "mouse-button"]).is_err());
        assert!(parse_buttons_set(&["4", "mouse-button", "x"]).is_err());
        assert!(parse_buttons_set(&["4", "macro"]).is_err());
        assert!(parse_buttons_set(&["4", "key"]).is_err());
        assert!(parse_buttons_set(&["4", "key", "pgup"]).is_err());
        assert!(parse_buttons_set(&["4", "key", "a", "--mods", "hyper"]).is_err());
        assert!(parse_buttons_set(&["4", "mouse-button", "4", "--mods", "ctrl"]).is_err());
        assert!(parse_buttons_set(&["4", "dpi-up", "2"]).is_err());
        let (a, _) = parse_buttons_set(&["4", "dpi-up", "--hypershift"]).unwrap();
        assert_eq!(a, buttons::dpi_up(1, 4, Mode::HyperShift));
        // The removed opt-in flag is rejected.
        assert!(
            parse_buttons_set(&["4", "dpi-up", "--hypershift", "--experimental-hypershift"])
                .is_err()
        );
    }

    #[test]
    fn buttons_list_parsing() {
        let cli = Cli::try_parse_from(["opensynapse", "buttons", "list"]).unwrap();
        let Command::Buttons {
            action: ButtonsAction::List { slot, offset },
        } = cli.command
        else {
            panic!("parsed wrong command")
        };
        assert_eq!((slot.profile, slot.mode(), offset), (1, Mode::Normal, 0));
        let cli = Cli::try_parse_from([
            "opensynapse",
            "buttons",
            "list",
            "--profile",
            "2",
            "--hypershift",
            "--offset",
            "16",
        ])
        .unwrap();
        let Command::Buttons {
            action: ButtonsAction::List { slot, offset },
        } = cli.command
        else {
            panic!("parsed wrong command")
        };
        assert_eq!(
            (slot.profile, slot.mode(), offset),
            (2, Mode::HyperShift, 16)
        );
        // Read-only: no write flags.
        assert!(Cli::try_parse_from(["opensynapse", "buttons", "list", "--apply"]).is_err());
    }

    #[test]
    fn list_lines_use_requested_mode() {
        let mut args = vec![
            1, 1, 0, 0x05, 0x01, 0x01, 0x04, 0, 0, 0, 0, 0x60, 0x06, 0x01, 0x06,
        ];
        args.resize(80, 0);
        let l = buttons::parse_button_assignment_list(&args).unwrap();
        assert_eq!(
            list_lines(1, Mode::HyperShift, &l),
            [
                "profile 1, button 5, HyperShift mode: mouse button 4 (function 1, data [4])",
                "profile 1, button 96, HyperShift mode: (function 6, data [6])",
            ]
        );
    }

    #[test]
    fn hex_trimming() {
        assert_eq!(hex_trimmed(&[0x01, 0xab, 0, 0]), "01 ab (+2 zero bytes)");
        assert_eq!(hex_trimmed(&[0x01, 0x02]), "01 02");
        assert_eq!(hex_trimmed(&[0, 0]), "(+2 zero bytes)");
    }

    #[test]
    fn buttons_get_defaults() {
        let cli = Cli::try_parse_from(["opensynapse", "buttons", "get", "3"]).unwrap();
        let Command::Buttons {
            action: ButtonsAction::Get { button, slot },
        } = cli.command
        else {
            panic!("parsed wrong command")
        };
        assert_eq!((button, slot.profile, slot.mode()), (3, 1, Mode::Normal));
    }

    #[test]
    fn button_restore_command() {
        assert_eq!(
            button_restore(&buttons::mouse_button(1, 4, Mode::Normal, 4)).as_deref(),
            Some("buttons set 4 mouse-button 4 --profile 1 --apply")
        );
        assert_eq!(
            button_restore(&buttons::double_click(3, 5, Mode::HyperShift, 1)).as_deref(),
            Some("buttons set 5 double-click 1 --profile 3 --hypershift --apply")
        );
        assert_eq!(
            button_restore(&buttons::keyboard(1, 4, Mode::Normal, 0x00, 0x04)).as_deref(),
            Some("buttons set 4 key a --profile 1 --apply")
        );
        assert_eq!(
            button_restore(&buttons::keyboard(1, 4, Mode::Normal, 0x05, 0x28)).as_deref(),
            Some("buttons set 4 key enter --mods ctrl,alt --profile 1 --apply")
        );
        assert_eq!(
            button_restore(&buttons::dpi_up(1, 4, Mode::Normal)).as_deref(),
            Some("buttons set 4 dpi-up --profile 1 --apply")
        );
        // Not expressible: unnamed usage, right-side modifier, unobserved DPI data,
        // unknown function.
        let raw = |function, data| ButtonAssignment {
            function,
            data,
            ..buttons::mouse_button(1, 4, Mode::Normal, 4)
        };
        assert_eq!(button_restore(&raw(2, vec![0, 0x50])), None);
        assert_eq!(button_restore(&raw(2, vec![0x10, 0x04])), None);
        assert_eq!(button_restore(&raw(6, vec![3])), None);
        assert_eq!(button_restore(&raw(9, vec![1])), None);
    }

    fn parse_profile(args: &[&str]) -> Result<ProfileAction> {
        let argv = ["opensynapse", "profile"]
            .into_iter()
            .chain(args.iter().copied());
        let Command::Profile { action } = Cli::try_parse_from(argv)?.command else {
            panic!("parsed wrong command")
        };
        Ok(action)
    }

    #[test]
    fn profile_parsing() {
        let ProfileAction::Export { buttons, out } = parse_profile(&["export"]).unwrap() else {
            panic!("parsed wrong action")
        };
        assert_eq!(buttons.0, [1, 2, 3, 4, 5]);
        assert_eq!(out, None);
        let ProfileAction::Export { buttons, out } =
            parse_profile(&["export", "--buttons", "4,6-7", "--out", "p.toml"]).unwrap()
        else {
            panic!("parsed wrong action")
        };
        assert_eq!(buttons.0, [4, 6, 7]);
        assert_eq!(out, Some(PathBuf::from("p.toml")));
        // The removed opt-in flag is rejected.
        assert!(parse_profile(&["export", "--hypershift"]).is_err());
        assert!(parse_profile(&["export", "--buttons", "0"]).is_err());
        assert!(parse_profile(&["export", "--apply"]).is_err());

        let ProfileAction::Apply {
            file,
            write,
            volatile,
            all,
        } = parse_profile(&["apply", "p.toml"]).unwrap()
        else {
            panic!("parsed wrong action")
        };
        assert_eq!(file, PathBuf::from("p.toml"));
        assert!(!write.apply && !write.force && !volatile && !all);
        let ProfileAction::Apply {
            write,
            volatile,
            all,
            ..
        } = parse_profile(&[
            "apply",
            "p.toml",
            "--apply",
            "--force",
            "--volatile",
            "--all",
        ])
        .unwrap()
        else {
            panic!("parsed wrong action")
        };
        assert!(write.apply && write.force && volatile && all);
        assert!(parse_profile(&["apply", "p.toml", "--experimental-hypershift"]).is_err());
        assert!(parse_profile(&["apply"]).is_err());
        // The printed restore hint parses back.
        let ProfileAction::Apply { file, write, .. } =
            parse_profile(&["apply", "dir/p.restore.toml", "--apply"]).unwrap()
        else {
            panic!("parsed wrong action")
        };
        assert_eq!(file, PathBuf::from("dir/p.restore.toml"));
        assert!(write.apply);
    }

    /// The restore command parses back to the old assignment.
    #[test]
    fn button_restore_roundtrips() {
        for old in [
            buttons::mouse_button(1, 4, Mode::Normal, 5),
            buttons::double_click(2, 6, Mode::HyperShift, 1),
            buttons::keyboard(1, 4, Mode::Normal, 0x00, 0x04),
            buttons::keyboard(3, 5, Mode::HyperShift, 0x0F, 0x45),
            buttons::dpi_up(1, 4, Mode::HyperShift),
        ] {
            let cmd = button_restore(&old).unwrap();
            let args: Vec<&str> = cmd.split(' ').skip(2).collect();
            assert_eq!(parse_buttons_set(&args).unwrap().0, old, "{cmd}");
        }
    }
}
