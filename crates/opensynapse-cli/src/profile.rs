//! `profile export` / `profile apply`: device state <-> `opensynapse_core::Profile`.
//!
//! The engine (reads, snapshot, skip-unchanged, writes with read-back) lives in
//! `opensynapse_device::profile_apply` and is shared with the agent. This module is
//! the CLI edge: argument parsing, the profile and restore files, and printing.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use opensynapse_core::Profile;
use opensynapse_device::profile_apply::{
    self as engine, ApplyError, ApplyOptions, ApplyPlan, limits_for,
};
use opensynapse_protocol::dpi;

use crate::{Target, WriteOpts};

/// Button IDs from `--buttons` (`1-5`, `1,2,4`, `1-3,6`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonList(pub Vec<u8>);

pub(crate) fn parse_button_list(s: &str) -> Result<ButtonList, String> {
    let num = |t: &str| -> Result<u8, String> {
        t.trim()
            .parse::<u8>()
            .ok()
            .filter(|&b| b >= 1)
            .ok_or_else(|| format!("`{}` is not a button ID (1-255)", t.trim()))
    };
    let mut out = Vec::new();
    for part in s.split(',') {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (num(a)?, num(b)?),
            None => {
                let n = num(part)?;
                (n, n)
            }
        };
        if a > b {
            return Err(format!("range `{}` runs backwards", part.trim()));
        }
        for id in a..=b {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    Ok(ButtonList(out))
}

/// Where the snapshot for `input` goes: `<stem>.restore.toml` next to it, or
/// `<stem>.restore-2.toml`, `-3`, … if that exists (an earlier snapshot is never
/// overwritten).
pub(crate) fn restore_path(input: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let is_toml = input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"));
    let stem = if is_toml {
        input.file_stem()
    } else {
        input.file_name()
    };
    let stem = stem.map_or("profile".into(), |s| s.to_string_lossy());
    let dir = input.parent().unwrap_or(Path::new(""));
    (1u32..)
        .map(|n| {
            dir.join(if n == 1 {
                format!("{stem}.restore.toml")
            } else {
                format!("{stem}.restore-{n}.toml")
            })
        })
        .find(|p| !exists(p))
        .expect("some free name")
}

/// Read and parse a profile file.
pub(crate) fn read_profile(file: &Path) -> Result<(String, Profile)> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let profile = Profile::from_toml(&text).with_context(|| file.display().to_string())?;
    Ok((text, profile))
}

/// Write a restore profile (with a comment header) to `path`.
pub(crate) fn write_restore(path: &Path, profile: &Profile, header: &str) -> Result<()> {
    let text = engine::render(profile, header)?;
    std::fs::write(path, text)
        .with_context(|| format!("cannot write the restore file {}", path.display()))
}

fn header(dev: &Target, what: &str) -> String {
    engine::header(dev.name(), dev.entry.pid, what)
}

/// `profile export`.
pub(crate) fn export(dev: &mut Target, ids: &ButtonList, out: Option<&Path>) -> Result<()> {
    let ex = engine::export(&mut dev.session, dev.entry.pid, &ids.0)?;
    for n in &ex.notes {
        eprintln!("note: {n}");
    }
    if let Err(e) = ex.profile.validate(&limits_for(dev.device)) {
        eprintln!("warning: the exported profile does not pass validation: {e}");
    }
    let text = engine::render(&ex.profile, &header(dev, "exported from"))?;
    match out {
        Some(path) => {
            std::fs::write(path, &text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!("Wrote {}.", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

/// Engine errors that name the profile get the file name, as before the move.
fn with_file(e: ApplyError, file: &Path) -> anyhow::Error {
    match e {
        ApplyError::Invalid(v) => anyhow::anyhow!("{}: {v}", file.display()),
        ApplyError::WrongDevice { profile, device } => anyhow::anyhow!(
            "{} is for PID {profile}, but the device is {device:04X}; use --force to apply anyway",
            file.display()
        ),
        other => other.into(),
    }
}

/// `profile apply`.
pub(crate) fn apply(
    dev: &mut Target,
    file: &Path,
    write: WriteOpts,
    volatile: bool,
    all: bool,
) -> Result<()> {
    let (_, profile) = read_profile(file)?;
    let storage = if volatile {
        dpi::Storage::Volatile
    } else {
        dpi::Storage::Persistent
    };
    let opts = ApplyOptions {
        storage,
        all,
        force: write.force,
    };
    let plan = engine::plan_apply(&mut dev.session, dev.entry.pid, dev.device, profile, opts)
        .map_err(|e| with_file(e, file))?;
    if plan.sets_nothing() {
        println!("{} sets nothing; no writes.", file.display());
        return Ok(());
    }
    for l in plan.unchanged_lines() {
        println!("{l}");
    }
    let Some(snap) = &plan.snapshot else {
        println!(
            "Nothing to change: the device already has all {} setting(s) of {}; no writes, no restore file.",
            plan.unchanged.len(),
            file.display()
        );
        return Ok(());
    };
    println!("{}", plan.summary_line());
    for n in &snap.notes {
        eprintln!("note: {n}");
    }
    let restore = restore_path(file, Path::exists);
    let hint = format!(
        "To restore: opensynapse --pid {:04X} profile apply {} --apply{}",
        dev.entry.pid,
        restore.display(),
        if volatile { " --volatile" } else { "" },
    );

    if !write.apply {
        for l in plan.dry_run_lines() {
            println!("{l}");
        }
        println!(
            "With --apply, the current values are saved to {} first.",
            restore.display()
        );
        println!("Run again with --apply to write.");
        return Ok(());
    }
    plan.check_snapshot()?;
    dev.guard(write)?;
    write_restore(
        &restore,
        &snap.profile,
        &header(dev, "values before applying to"),
    )?;
    println!("Saved the current values to {}.", restore.display());
    println!("{hint}");
    run_writes(dev, &plan, &hint)?;
    println!(
        "Applied {} write(s) from {}; {} unchanged.",
        plan.writes.len(),
        file.display(),
        plan.unchanged.len()
    );
    Ok(())
}

/// Send the writes, printing progress; on failure print where it stopped and how to
/// restore, then return the write's own error.
fn run_writes(dev: &mut Target, plan: &ApplyPlan, hint: &str) -> Result<()> {
    match engine::execute(&mut dev.session, plan, |l| println!("{l}")) {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Some(l) = e.stopped_line() {
                println!("{l}");
            }
            println!("{hint}");
            match e {
                ApplyError::Write { source, .. } => Err(source.into()),
                other => Err(other.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_list_parsing() {
        assert_eq!(
            parse_button_list("1-5"),
            Ok(ButtonList(vec![1, 2, 3, 4, 5]))
        );
        assert_eq!(parse_button_list("4"), Ok(ButtonList(vec![4])));
        assert_eq!(
            parse_button_list("1-3, 6,2"),
            Ok(ButtonList(vec![1, 2, 3, 6]))
        );
        for bad in ["", "0", "5-1", "x", "1-", "256"] {
            assert!(parse_button_list(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn restore_file_name() {
        let none = |_: &Path| false;
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            restore_path(&p("dir/gaming.toml"), none),
            p("dir/gaming.restore.toml")
        );
        assert_eq!(
            restore_path(&p("gaming.TOML"), none),
            p("gaming.restore.toml")
        );
        assert_eq!(restore_path(&p("gaming"), none), p("gaming.restore.toml"));
        assert_eq!(restore_path(&p("a.b.txt"), none), p("a.b.txt.restore.toml"));
        // Applying a restore file snapshots to a new file, not over itself.
        assert_eq!(
            restore_path(&p("gaming.restore.toml"), none),
            p("gaming.restore.restore.toml")
        );
        // Earlier snapshots are kept.
        let taken = [p("g.restore.toml"), p("g.restore-2.toml")];
        assert_eq!(
            restore_path(&p("g.toml"), |x| taken.iter().any(|t| t == x)),
            p("g.restore-3.toml")
        );
    }

    #[test]
    fn errors_name_the_file() {
        let e = with_file(
            ApplyError::WrongDevice {
                profile: opensynapse_core::Pid(0x00B9),
                device: 0x00B8,
            },
            Path::new("g.toml"),
        );
        assert_eq!(
            e.to_string(),
            "g.toml is for PID 00B9, but the device is 00B8; use --force to apply anyway"
        );
        let e = with_file(
            ApplyError::SnapshotIncomplete(vec!["a".into(), "b".into()]),
            Path::new("g.toml"),
        );
        assert_eq!(
            e.to_string(),
            "the snapshot taken before writing could not undo this profile:\n  a\n  b\n\
             Nothing was written. Use --force to apply anyway."
        );
    }
}
