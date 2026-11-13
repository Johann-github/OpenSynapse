//! `opensynapse report`: a Markdown block about every connected Razer device, to
//! paste into a "Device report" issue (`.github/ISSUE_TEMPLATE/device-report.yml`).
//! Maintainers use it to mark devices `tested = true`
//! (`vault/Devices/Device Registry.md`, section "Community device reports").
//!
//! Read-only: it enumerates HID collections, opens the control collection and
//! sends only GET commands. It never writes to a device and never touches the
//! network. Every read is independent: a failure is printed as that field's
//! result and the report goes on. Serial numbers are masked. Raw HID paths are
//! not printed (they can carry instance IDs or Bluetooth addresses); only the
//! `MI_xx COLyy` part is.
//!
//! Gathering ([`gather`]) talks to the device; rendering ([`render`]) is pure.

use std::fmt::Write as _;

use anyhow::Result;
use opensynapse_device_registry::{Device, Match, Protocol, RAZER_VID, Registry};
use opensynapse_hid::{Collection, DEFAULT_INTERFACE, Timing, Transport, is_no_response_text};
use opensynapse_protocol::buttons::Mode;
use opensynapse_protocol::{device, dpi, power};

use crate::{Target, hid_probe};

/// Buttons whose normal-mode assignment the report reads (onboard profile 1).
const BUTTONS: std::ops::RangeInclusive<u8> = 1..=5;

/// Everything the report shows.
#[derive(Debug)]
pub struct Report {
    pub version: String,
    pub os: String,
    pub arch: String,
    /// Running Razer software that may talk to the device at the same time.
    pub razer_software: Option<String>,
    /// HID enumeration failed (then `devices` is empty).
    pub enumeration_error: Option<String>,
    pub devices: Vec<DeviceReport>,
}

/// One connected PID.
#[derive(Debug)]
pub struct DeviceReport {
    pub vid: u16,
    pub pid: u16,
    pub registry: Option<RegistryInfo>,
    pub collections: Vec<Collection>,
    pub control: Control,
    /// Why the reads were not attempted (`reads` is empty then).
    pub reads_skipped: Option<String>,
    pub reads: Vec<Field>,
}

/// What the device registry knows about a PID.
#[derive(Debug)]
pub struct RegistryInfo {
    pub name: String,
    pub kind: String,
    /// Protocol on the matched PID (a receiver PID can differ from the product).
    pub protocol: Protocol,
    pub tested: bool,
    /// `primary`, `variant` or `dongle`.
    pub matched: &'static str,
}

/// The protocol 2.5 control collection (usage 0001/0002, 91-byte feature report).
#[derive(Debug)]
pub enum Control {
    NotFound,
    Opened,
    OpenFailed(String),
}

/// One read: label and value, or the error text.
#[derive(Debug)]
pub struct Field {
    pub label: String,
    pub value: Result<String, String>,
}

/// Gather and print the report. `pid` is the global `--pid`.
pub fn run(pid: Option<u16>) -> Result<()> {
    print!("{}", render(&gather(pid)));
    Ok(())
}

/// Talk to the devices. Never fails as a whole; problems end up in the report.
pub fn gather(pid: Option<u16>) -> Report {
    let mut report = Report {
        version: env!("CARGO_PKG_VERSION").into(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        razer_software: opensynapse_hid::conflicting_process().map(Into::into),
        enumeration_error: None,
        devices: Vec::new(),
    };
    let cols = match opensynapse_hid::collections(Some(RAZER_VID)) {
        Ok(c) => c,
        Err(e) => {
            report.enumeration_error = Some(e.to_string());
            return report;
        }
    };
    let mut pids: Vec<u16> = cols
        .iter()
        .map(|c| c.entry.pid)
        .filter(|p| pid.is_none_or(|want| *p == want))
        .collect();
    pids.dedup(); // `collections` is sorted by PID.
    // Opening needs a `Transport`; a failure only costs the reads.
    let transport = Transport::new().map_err(|e| e.to_string());
    let reg = Registry::embedded();
    for p in pids {
        let collections: Vec<Collection> =
            cols.iter().filter(|c| c.entry.pid == p).cloned().collect();
        let m = reg.lookup(RAZER_VID, p);
        let mut dev = DeviceReport {
            vid: RAZER_VID,
            pid: p,
            registry: m.map(registry_info),
            collections,
            control: Control::NotFound,
            reads_skipped: None,
            reads: Vec::new(),
        };
        let control = dev
            .collections
            .iter()
            .find(|c| c.entry.is_control(DEFAULT_INTERFACE))
            .map(|c| c.entry.clone());
        let protocol = m.map_or(Protocol::Unknown, |m| m.protocol());
        match (control, &transport) {
            (None, _) => {
                dev.reads_skipped = Some("no protocol 2.5 control collection".into());
            }
            (Some(_), Err(e)) => {
                dev.control = Control::OpenFailed(e.clone());
                dev.reads_skipped = Some("control collection not opened".into());
            }
            (Some(entry), Ok(t)) => match t.open(&entry, report_timing()) {
                Err(e) => {
                    dev.control = Control::OpenFailed(e.to_string());
                    dev.reads_skipped = Some("control collection not opened".into());
                }
                Ok(session) => {
                    dev.control = Control::Opened;
                    if let Some(why) = skip_reads(protocol) {
                        dev.reads_skipped = Some(why);
                    } else {
                        let mut target = Target {
                            entry,
                            device: m.map(|m: Match<'static>| m.device()),
                            session,
                            conflict: None,
                        };
                        dev.reads = read_all(&mut target);
                    }
                }
            },
        }
        report.devices.push(dev);
    }
    report
}

/// Fewer resends than normal commands: a device that does not answer would
/// otherwise stall every field. Polling while busy is unchanged.
fn report_timing() -> Timing {
    Timing {
        max_out: 5,
        ..Timing::default()
    }
}

/// Reads are 2.5 GET commands and go to 2.5 PIDs only. `unknown` means "do not
/// talk to it yet" (`vault/Devices/Device Registry.md`, detection recipe);
/// 3.0/4.0 PIDs wait for their transports. The collection table still shows
/// whether such a PID has a 2.5-shaped control collection.
fn skip_reads(protocol: Protocol) -> Option<String> {
    match protocol {
        Protocol::V25 => None,
        Protocol::Unknown => Some("protocol unknown; OpenSynapse does not talk to it yet".into()),
        p => Some(format!("protocol {p} is not implemented yet")),
    }
}

fn registry_info(m: Match<'static>) -> RegistryInfo {
    let d: &Device = m.device();
    RegistryInfo {
        name: d.name.clone(),
        kind: format!("{:?}", d.kind).to_lowercase(),
        protocol: m.protocol(),
        tested: d.hardware_tested(),
        matched: match m {
            Match::Primary(_) => "primary",
            Match::Variant(..) => "variant",
            Match::Dongle(_) => "dongle",
        },
    }
}

/// Every safe read, each on its own; errors become the field's value.
fn read_all(dev: &mut Target) -> Vec<Field> {
    let mut out = Vec::new();
    let mut push = |label: &str, r: Result<String>| {
        out.push(Field {
            label: label.into(),
            value: r.map_err(|e| format!("{e:#}")),
        });
    };
    push(
        "Firmware",
        dev.query(
            &device::get_firmware_version(),
            device::parse_firmware_version,
        )
        .map(|v| v.to_string()),
    );
    push(
        "Serial (masked)",
        dev.query(&device::get_serial(), device::parse_serial)
            .map(|s| mask_serial(&s)),
    );
    push(
        "Device mode",
        dev.query(&device::get_device_mode(), device::parse_device_mode)
            .map(|(m, p)| format!("{m:?} (param {p})")),
    );
    push(
        "Battery",
        dev.query(&power::get_battery_level(), power::parse_battery_level)
            .map(|l| format!("{}% (raw {})", l.percent(), l.0)),
    );
    push(
        "Charging",
        dev.query(&power::get_charging_status(), power::parse_charging_status)
            .map(|c| c.to_string()),
    );
    push(
        "DPI",
        dev.query(&dpi::get_dpi(dpi::Storage::Volatile), dpi::parse_dpi)
            .map(|d| d.to_string()),
    );
    push(
        "DPI stages",
        dev.query(
            &dpi::get_dpi_stages(dpi::Storage::Volatile),
            dpi::parse_dpi_stages,
        )
        .map(|s| stages_line(&s)),
    );
    push(
        "Polling rate",
        crate::polling_get(dev, None, 0)
            .map(|(hz, v)| format!("{hz} Hz ({})", crate::variant_name(v))),
    );
    push(
        "Sleep timer",
        dev.query(&power::get_sleep_time(), power::parse_sleep_time)
            .map(|v| format!("{v} s")),
    );
    for b in BUTTONS {
        push(
            &format!("Button {b} (profile 1, normal)"),
            crate::read_button(dev, 1, b, Mode::Normal).map(|r| r.to_string()),
        );
    }
    out
}

/// `400, *800, 1600`: the active stage carries `*`.
fn stages_line(s: &dpi::DpiStages) -> String {
    if s.stages.is_empty() {
        return "none".into();
    }
    s.stages
        .iter()
        .map(|st| {
            let mark = if st.id == s.active { "*" } else { "" };
            format!("{mark}{}", st.dpi)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Only the last 4 characters stay visible; shorter serials are fully masked.
pub fn mask_serial(serial: &str) -> String {
    let chars: Vec<char> = serial.chars().collect();
    if chars.is_empty() {
        return "(empty)".into();
    }
    let keep = if chars.len() > 4 { 4 } else { 0 };
    let hidden = chars.len() - keep;
    let mut s = "*".repeat(hidden);
    s.extend(&chars[hidden..]);
    s
}

/// Table cell text: no line breaks, `|` escaped.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn len(n: Option<u16>) -> String {
    n.map_or_else(|| "?".into(), |n| n.to_string())
}

/// The Markdown block (pure).
pub fn render(r: &Report) -> String {
    let mut s = String::from("### OpenSynapse device report\n\n");
    let _ = writeln!(s, "- OpenSynapse: {}", r.version);
    let _ = writeln!(s, "- OS: {} ({})", r.os, r.arch);
    let _ = writeln!(
        s,
        "- Razer software running: {}",
        r.razer_software.as_deref().map_or_else(
            || "no".to_string(),
            |n| format!("{n} (may interfere with the reads)")
        )
    );
    s.push_str(
        "\nRead-only: nothing was written to the device. Serial numbers are masked \
         (only the last 4 characters are shown); HID paths are shortened.\n",
    );
    if let Some(e) = &r.enumeration_error {
        let _ = writeln!(s, "\nHID enumeration failed: {}", cell(e));
        return s;
    }
    if r.devices.is_empty() {
        s.push_str("\nNo Razer HID devices found.\n");
        return s;
    }
    for d in &r.devices {
        render_device(&mut s, d);
    }
    s
}

fn render_device(s: &mut String, d: &DeviceReport) {
    let name = d
        .registry
        .as_ref()
        .map_or("unknown Razer device", |i| i.name.as_str());
    let _ = writeln!(s, "\n#### {:04X}:{:04X} {name}\n", d.vid, d.pid);
    match &d.registry {
        Some(i) => {
            let _ = writeln!(
                s,
                "- Registry: {}, protocol {}, tested: {} (matched {} PID)",
                i.kind,
                i.protocol,
                if i.tested { "yes" } else { "no" },
                i.matched
            );
        }
        None => s.push_str("- Registry: not listed\n"),
    }
    let control = match &d.control {
        Control::NotFound => "not found".to_string(),
        Control::Opened => "opened".to_string(),
        Control::OpenFailed(e) => format!("open failed: {}", cell(e)),
    };
    let _ = writeln!(s, "- Control collection: {control}");

    s.push_str("\nHID collections:\n\n");
    s.push_str("| Interface | Collection | Usage page/usage | In | Out | Feature | Readable |\n");
    s.push_str("|---|---|---|---|---|---|---|\n");
    for c in &d.collections {
        let e = &c.entry;
        let _ = writeln!(
            s,
            "| {} | {} | {:04X}/{:04X} | {} | {} | {} | {} |",
            e.interface,
            cell(&hid_probe::short_path(&e.path)),
            e.usage_page,
            e.usage,
            len(e.input_len),
            len(e.output_len),
            len(e.feature_len),
            if c.readable { "yes" } else { "no" }
        );
    }

    if let Some(why) = &d.reads_skipped {
        let _ = writeln!(s, "\nReads: skipped ({})", cell(why));
        return;
    }
    s.push_str("\nReads:\n\n| Setting | Result |\n|---|---|\n");
    for f in &d.reads {
        let value = match &f.value {
            Ok(v) => cell(v),
            Err(e) => format!("error: {}", cell(e)),
        };
        let _ = writeln!(s, "| {} | {value} |", cell(&f.label));
    }
    if d.reads
        .iter()
        .any(|f| f.value.as_ref().is_err_and(|e| is_no_response_text(e)))
    {
        s.push_str(
            "\nSome reads got no response: a wireless device may be asleep. Move or wake it \
             and run `opensynapse report` again.\n",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensynapse_hid::HidEntry;

    fn col(path: &str, interface: i32, up: u16, u: u16, feat: u16) -> Collection {
        Collection {
            entry: HidEntry {
                vid: 0x1532,
                pid: 0x00B8,
                interface,
                usage_page: up,
                usage: u,
                feature_len: Some(feat),
                input_len: Some(9),
                output_len: Some(0),
                path: path.into(),
            },
            readable: false,
        }
    }

    fn base(devices: Vec<DeviceReport>) -> Report {
        Report {
            version: "0.1.0".into(),
            os: "windows".into(),
            arch: "x86_64".into(),
            razer_software: None,
            enumeration_error: None,
            devices,
        }
    }

    fn viper() -> DeviceReport {
        DeviceReport {
            vid: 0x1532,
            pid: 0x00B8,
            registry: Some(RegistryInfo {
                name: "Razer Viper V3 HyperSpeed".into(),
                kind: "mouse".into(),
                protocol: Protocol::V25,
                tested: true,
                matched: "primary",
            }),
            collections: vec![col(
                r"\\?\hid#vid_1532&pid_00b8&mi_00&col02#7&secret&0&0001#{4d1e}",
                0,
                0x0001,
                0x0002,
                91,
            )],
            control: Control::Opened,
            reads_skipped: None,
            reads: vec![
                Field {
                    label: "Firmware".into(),
                    value: Ok("1.02".into()),
                },
                Field {
                    label: "Serial (masked)".into(),
                    value: Ok(mask_serial("PM2345H12345678")),
                },
                Field {
                    label: "Battery".into(),
                    value: Err("device answered NotSupported | x\nmore".into()),
                },
            ],
        }
    }

    #[test]
    fn serial_is_masked() {
        assert_eq!(mask_serial("PM2345H12345678"), "***********5678");
        assert_eq!(mask_serial("ABCD"), "****");
        assert_eq!(mask_serial("AB"), "**");
        assert_eq!(mask_serial(""), "(empty)");
    }

    #[test]
    fn header_and_device() {
        let out = render(&base(vec![viper()]));
        assert!(out.starts_with("### OpenSynapse device report\n\n- OpenSynapse: 0.1.0\n"));
        assert!(out.contains("- OS: windows (x86_64)\n"));
        assert!(out.contains("- Razer software running: no\n"));
        assert!(out.contains("Serial numbers are masked"));
        assert!(out.contains("\n#### 1532:00B8 Razer Viper V3 HyperSpeed\n"));
        assert!(out.contains("- Registry: mouse, protocol 2.5, tested: yes (matched primary PID)"));
        assert!(out.contains("- Control collection: opened\n"));
        assert!(out.contains("| 0 | MI_00 COL02 | 0001/0002 | 9 | 0 | 91 | no |\n"));
        assert!(out.contains("| Firmware | 1.02 |\n"));
        assert!(out.contains("| Serial (masked) | ***********5678 |\n"));
        // Errors stay on one table row with `|` escaped.
        assert!(out.contains("| Battery | error: device answered NotSupported \\| x more |\n"));
        // Neither the full serial nor the raw path leak.
        assert!(!out.contains("PM2345H1"));
        assert!(!out.contains("secret"));
        assert!(!out.contains("no response"));
        let mut d = viper();
        d.reads[2].value = Err(format!("{} (status Timeout)", opensynapse_hid::NO_RESPONSE));
        assert!(render(&base(vec![d])).contains("may be asleep"));
    }

    #[test]
    fn unknown_device_without_control() {
        let d = DeviceReport {
            vid: 0x1532,
            pid: 0x0FFF,
            registry: None,
            collections: vec![col("plain", 1, 0x000C, 0x0001, 0)],
            control: Control::NotFound,
            reads_skipped: Some("no protocol 2.5 control collection".into()),
            reads: Vec::new(),
        };
        let out = render(&base(vec![d]));
        assert!(out.contains("#### 1532:0FFF unknown Razer device\n"));
        assert!(out.contains("- Registry: not listed\n"));
        assert!(out.contains("- Control collection: not found\n"));
        assert!(out.contains("Reads: skipped (no protocol 2.5 control collection)\n"));
        assert!(!out.contains("| Setting |"));
    }

    #[test]
    fn empty_and_failed_enumeration() {
        let mut r = base(Vec::new());
        r.razer_software = Some("RazerAppEngine.exe".into());
        let out = render(&r);
        assert!(out.contains("No Razer HID devices found."));
        assert!(out.contains("RazerAppEngine.exe (may interfere"));
        r.enumeration_error = Some("access denied".into());
        assert!(render(&r).contains("HID enumeration failed: access denied"));
    }

    #[test]
    fn open_failure_is_reported() {
        let mut d = viper();
        d.control = Control::OpenFailed("Zugriff verweigert".into());
        d.reads_skipped = Some("control collection not opened".into());
        d.reads.clear();
        let out = render(&base(vec![d]));
        assert!(out.contains("- Control collection: open failed: Zugriff verweigert\n"));
        assert!(out.contains("Reads: skipped (control collection not opened)"));
    }

    #[test]
    fn reads_only_for_25() {
        assert_eq!(skip_reads(Protocol::V25), None);
        assert!(skip_reads(Protocol::Unknown).is_some());
        assert_eq!(
            skip_reads(Protocol::V30).as_deref(),
            Some("protocol 3.0 is not implemented yet")
        );
        assert!(skip_reads(Protocol::V40).is_some());
    }

    #[test]
    fn stages_mark_active() {
        let s = dpi::DpiStages {
            active: 2,
            stages: vec![
                dpi::DpiStage {
                    id: 1,
                    dpi: dpi::Dpi { x: 400, y: 400 },
                },
                dpi::DpiStage {
                    id: 2,
                    dpi: dpi::Dpi { x: 800, y: 800 },
                },
            ],
        };
        let line = stages_line(&s);
        assert!(line.contains(", *"), "{line}");
        assert!(!line.starts_with('*'), "{line}");
    }
}
