//! `opensynapse hid …`: diagnostics for finding the collection that carries the
//! device's hardware events (input report ID 5, `vault/Protocol/Hardware Events.md`).
//!
//! `collections` only opens and closes handles. `listen` reads input reports; it
//! never sends anything to the device. Output is built by pure functions so it
//! can be tested without hardware.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::Subcommand;
use opensynapse_device_registry::RAZER_VID;
use opensynapse_hid::{Collection, HidEntry, input};
use opensynapse_protocol::event;

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum HidAction {
    /// List HID collections with usage page/usage, report lengths and whether
    /// they can be opened for reading. Razer only unless --all-vendors.
    Collections {
        /// Every HID collection on the system, not only VID 1532.
        #[arg(long)]
        all_vendors: bool,
    },
    /// Print every input report of the chosen collection(s) for a while. Read-only:
    /// nothing is sent to the device. Notification records (ID 5 / 9) are decoded.
    Listen {
        /// Collection path, as shown by `hid collections`. Repeatable.
        #[arg(long, required_unless_present = "all_readable")]
        path: Vec<String>,
        /// Every readable Razer collection with an input report (narrow with --pid).
        #[arg(long, conflicts_with = "path")]
        all_readable: bool,
        /// How long to listen.
        #[arg(long, default_value_t = 15)]
        seconds: u64,
    },
}

/// Run one `hid` command. `pid` is the global `--pid`.
pub fn run(action: HidAction, pid: Option<u16>) -> Result<()> {
    match action {
        HidAction::Collections { all_vendors } => {
            let vid = (!all_vendors).then_some(RAZER_VID);
            let cols = opensynapse_hid::collections(vid)?;
            let cols: Vec<&Collection> = cols
                .iter()
                .filter(|c| pid.is_none_or(|p| c.entry.pid == p))
                .collect();
            print!("{}", render_table(&cols));
            Ok(())
        }
        HidAction::Listen {
            path,
            all_readable,
            seconds,
        } => {
            let cols = opensynapse_hid::collections(Some(RAZER_VID))?;
            let chosen: Vec<(String, String)> = if all_readable {
                listenable(&cols, pid)
                    .into_iter()
                    .map(|c| (label(&c.entry), c.entry.path.clone()))
                    .collect()
            } else {
                path.into_iter()
                    .map(|p| {
                        let l = cols
                            .iter()
                            .find(|c| c.entry.path.eq_ignore_ascii_case(&p))
                            .map_or_else(|| short_path(&p), |c| label(&c.entry));
                        (l, p)
                    })
                    .collect()
            };
            listen(&chosen, Duration::from_secs(seconds))
        }
    }
}

fn listen(chosen: &[(String, String)], dur: Duration) -> Result<()> {
    if chosen.is_empty() {
        bail!("no readable collection with an input report (see `opensynapse hid collections`)");
    }
    let start = Instant::now();
    let mut readers = Vec::new();
    for (lbl, path) in chosen {
        let tag = lbl.clone();
        match input::spawn(path, move |raw| {
            print!("{}", render_report(start.elapsed(), &tag, raw));
            true
        }) {
            Ok(h) => {
                eprintln!("listening on {lbl}");
                readers.push((lbl, h));
            }
            Err(e) => eprintln!("cannot read {lbl}: {e}"),
        }
    }
    if readers.is_empty() {
        bail!("none of the chosen collections could be opened for reading");
    }
    eprintln!(
        "{} reader(s) for {} s; press the DPI button, lift the mouse, plug/unplug the cable …",
        readers.len(),
        dur.as_secs()
    );
    // One deadline wait; the readers block in the kernel meanwhile.
    std::thread::sleep(dur);
    for (lbl, h) in readers {
        if let Err(e) = h.stop() {
            eprintln!("{lbl}: reader ended early: {e}");
        }
    }
    Ok(())
}

/// Readable Razer collections that have an input report, optionally for one PID.
pub fn listenable(cols: &[Collection], pid: Option<u16>) -> Vec<&Collection> {
    cols.iter()
        .filter(|c| c.readable && c.entry.input_len != Some(0))
        .filter(|c| pid.is_none_or(|p| c.entry.pid == p))
        .collect()
}

/// `MI_01 COL05` style part of an interface path (empty parts omitted).
pub fn short_path(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    let part = |key: &str| {
        lower
            .find(key)
            .and_then(|i| lower.get(i + 1..i + key.len() + 2))
            .map(str::to_ascii_uppercase)
    };
    let parts: Vec<String> = [part("&mi_"), part("&col")].into_iter().flatten().collect();
    if parts.is_empty() {
        path.to_owned()
    } else {
        parts.join(" ")
    }
}

/// Human label of a collection: PID, interface/collection, usage page/usage.
pub fn label(e: &HidEntry) -> String {
    format!(
        "{:04X} {} {:04X}/{:04X}",
        e.pid,
        short_path(&e.path),
        e.usage_page,
        e.usage
    )
}

pub fn hex(raw: &[u8]) -> String {
    let mut s = String::with_capacity(raw.len() * 3);
    for (i, b) in raw.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02X}");
    }
    s
}

/// One report line, plus a decode line for notification records.
pub fn render_report(t: Duration, label: &str, raw: &[u8]) -> String {
    let mut s = format!("t+{:>6}ms  {label}  {}\n", t.as_millis(), hex(raw));
    if matches!(
        raw.first(),
        Some(&(event::RECORD_NOTIFICATION | event::RECORD_SECONDARY_NOTIFICATION))
    ) {
        match event::decode(raw) {
            Some(n) => {
                let _ = writeln!(s, "           -> {n:?}");
            }
            None => s.push_str("           -> (notification record, not decodable)\n"),
        }
    }
    s
}

fn len(n: Option<u16>) -> String {
    n.map_or_else(|| "?".into(), |n| n.to_string())
}

/// The `hid collections` table.
pub fn render_table(cols: &[&Collection]) -> String {
    if cols.is_empty() {
        return "no HID collections found\n".into();
    }
    let mut s = String::from("VID  PID  IF  UP   U     IN  OUT FEAT READ  PATH\n");
    for c in cols {
        let e = &c.entry;
        let _ = writeln!(
            s,
            "{:04X} {:04X} {:>2}  {:04X} {:04X} {:>4} {:>4} {:>4} {:<4}  {}",
            e.vid,
            e.pid,
            e.interface,
            e.usage_page,
            e.usage,
            len(e.input_len),
            len(e.output_len),
            len(e.feature_len),
            if c.readable { "yes" } else { "no" },
            e.path
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(path: &str, up: u16, u: u16, input: u16, readable: bool) -> Collection {
        Collection {
            entry: HidEntry {
                vid: 0x1532,
                pid: 0x00B8,
                interface: 1,
                usage_page: up,
                usage: u,
                feature_len: Some(0),
                input_len: Some(input),
                output_len: Some(0),
                path: path.into(),
            },
            readable,
        }
    }

    #[test]
    fn short_path_parts() {
        assert_eq!(
            short_path(r"\\?\hid#vid_1532&pid_00b8&mi_01&col05#8&x&0&0004#{4d1e}"),
            "MI_01 COL05"
        );
        assert_eq!(
            short_path(r"\\?\hid#vid_1532&pid_00b8&mi_00#7&x#{4d1e}"),
            "MI_00"
        );
        assert_eq!(short_path("plain"), "plain");
    }

    #[test]
    fn report_line_and_decode() {
        let s = render_report(
            Duration::from_millis(42),
            "00B8 MI_01 FF00/0001",
            &[5, 0x02, 0x03, 0x20, 0x03, 0x20],
        );
        let mut lines = s.lines();
        assert_eq!(
            lines.next().unwrap(),
            "t+    42ms  00B8 MI_01 FF00/0001  05 02 03 20 03 20"
        );
        assert!(
            lines
                .next()
                .unwrap()
                .contains("DpiChanged { x: 800, y: 800 }")
        );
        // Plain mouse report: no decode line.
        assert_eq!(
            render_report(Duration::ZERO, "l", &[1, 0, 0])
                .lines()
                .count(),
            1
        );
        // Truncated notification is flagged.
        assert!(render_report(Duration::ZERO, "l", &[5]).contains("not decodable"));
    }

    #[test]
    fn selection_keeps_readable_with_input() {
        let cols = vec![
            col("a", 0xFF00, 1, 64, true),
            col("b", 0x0001, 2, 9, false),
            col("c", 0x000C, 1, 0, true),
        ];
        let got: Vec<&str> = listenable(&cols, None)
            .iter()
            .map(|c| c.entry.path.as_str())
            .collect();
        assert_eq!(got, ["a"]);
        assert!(listenable(&cols, Some(0x0001)).is_empty());
    }

    #[test]
    fn table_rows() {
        let a = col(
            r"\\?\hid#vid_1532&pid_00b8&mi_01&col05#x",
            0xFF00,
            1,
            64,
            true,
        );
        let t = render_table(&[&a]);
        let row = t.lines().nth(1).unwrap();
        assert!(
            row.starts_with("1532 00B8  1  FF00 0001   64    0    0 yes"),
            "{row}"
        );
        assert_eq!(render_table(&[]), "no HID collections found\n");
        assert_eq!(hex(&[]), "");
        assert_eq!(hex(&[0, 0xAB]), "00 AB");
    }
}
