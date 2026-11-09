//! CLI dispatch. See README.md.
//!
//! Reads are always safe. Writes are dry runs unless `--apply` is given; they show
//! the current value first, write, read back, and print how to restore.

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use opensynapse_device_registry::{Device, Match, Protocol, RAZER_VID, Registry};
use opensynapse_hid::{DEFAULT_INTERFACE, DeviceSession, HidEntry, HidError, Timing, Transport};
use opensynapse_protocol::{Request, Status, device, dpi, polling, power};

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
    /// Show the DPI stages stored on the device.
    DpiStages,
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
        /// Store on the device (storage byte 1) instead of volatile (0). Meaning inferred.
        #[arg(long)]
        persistent: bool,
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
        /// Raw value (unit unconfirmed, likely seconds).
        value: u16,
        #[command(flatten)]
        write: WriteOpts,
    },
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

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum PollingVariant {
    Period,
    Hyper,
}

fn parse_hex_u16(s: &str) -> Result<u16, String> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(t, 16).map_err(|e| format!("not a hex PID: {e}"))
}

pub fn run(cli: Cli) -> Result<()> {
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("debug")
            .with_writer(std::io::stderr)
            .init();
    }
    let transport = Transport::new().context("cannot enumerate HID devices")?;
    if let Command::List = cli.command {
        return list(&transport);
    }

    let mut dev = Target::select(&transport, cli.pid)?;
    match cli.command {
        Command::List => unreachable!(),
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
        Command::DpiStages => {
            let s = dev.query(
                &dpi::get_dpi_stages(dpi::Storage::Volatile),
                dpi::parse_dpi_stages,
            )?;
            for st in &s.stages {
                let mark = if st.id == s.active { "*" } else { " " };
                println!("{mark} stage {}: {}", st.id, st.dpi);
            }
            Ok(())
        }
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
                println!("Sleep timer: {v} (raw; unit unconfirmed, likely seconds)");
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
                    verify("sleep timer", now, value)?;
                }
                Ok(())
            }
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
    let storage = if persistent {
        dpi::Storage::Persistent
    } else {
        dpi::Storage::Volatile
    };
    let old = dev.query(&dpi::get_dpi(storage), dpi::parse_dpi)?;
    let new = dpi::Dpi { x, y };
    dev.write(
        write,
        &format!("DPI {old} -> {new}"),
        &format!("dpi set {} --y {} --apply", old.x, old.y),
        &dpi::set_dpi(storage, new),
    )?;
    if write.apply {
        verify(
            "DPI",
            dev.query(&dpi::get_dpi(storage), dpi::parse_dpi)?,
            new,
        )?;
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
    let req = match v {
        PollingVariant::Period => polling::set_polling_rate(hz)?,
        PollingVariant::Hyper => polling::set_hyper_polling(profile, hz)?,
    };
    dev.write(
        write,
        &format!("polling rate {old} Hz -> {hz} Hz ({})", variant_name(v)),
        &format!("polling set {old} --variant {} --apply", variant_name(v)),
        &req,
    )?;
    if write.apply {
        verify("polling rate", polling_get(dev, Some(v), profile)?.0, hz)?;
    }
    Ok(())
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

fn verify<T: PartialEq + std::fmt::Display>(what: &str, now: T, want: T) -> Result<()> {
    if now == want {
        println!("Verified: {what} is now {now}.");
        Ok(())
    } else {
        bail!("{what} reads back as {now}, expected {want}")
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
        parse: fn(&[u8]) -> Result<T, opensynapse_protocol::ProtocolError>,
    ) -> Result<T> {
        let reply = self.session.execute(req)?;
        Ok(parse(reply.args())?)
    }

    /// Dry run unless `--apply`; refuses while Razer software runs unless `--force`.
    fn write(&mut self, opts: WriteOpts, change: &str, restore: &str, req: &Request) -> Result<()> {
        if !opts.apply {
            println!("Dry run: would change {change}.");
            println!("Report: {:?}", req.encode(0));
            println!("Run again with --apply to write.");
            return Ok(());
        }
        if let Some(name) = self.conflict
            && !opts.force
        {
            bail!("{name} is running; close it or pass --force");
        }
        self.session.execute(req)?;
        println!("Changed {change}.");
        println!(
            "To restore: opensynapse --pid {:04X} {restore}",
            self.entry.pid
        );
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
}
