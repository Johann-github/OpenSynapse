//! `opensynapse install` / `uninstall`: copy the binaries to the fixed per-user
//! folder (`%LOCALAPPDATA%\Programs\OpenSynapse`; Linux `~/.local/bin`) and
//! point autostart there, so rebuilding `target\release` never collides with
//! the running agent (`vault/Decisions/ADR-0009 Install location.md`,
//! `vault/Decisions/ADR-0011 Linux autostart and install.md`).
//!
//! What to copy, delete and do with autostart is decided by the pure functions
//! in `opensynapse_sys::install`; this module does the I/O and prints each step.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use opensynapse_ipc::Client;
use opensynapse_ipc::msg::{Reply, Request};
use opensynapse_sys::install::{self, AutostartPlan, Replaced};
use opensynapse_sys::{AGENT_EXE, GUI_EXE, Process, autostart, config, shortcut};

/// How long to wait for a stopped agent to exit (it answers `Shutdown` first,
/// then closes its sessions). Blocks on the process handle, no polling.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// `--dir`, else the default install folder.
fn target_dir(dir: Option<PathBuf>) -> Result<PathBuf> {
    dir.or_else(install::dir).ok_or_else(|| {
        anyhow!(
            "cannot determine the install folder ({} is not set); pass --dir",
            install::DIR_ROOT_VAR
        )
    })
}

/// The folder of the running CLI, where the build put all binaries.
fn exe_dir() -> Result<PathBuf> {
    let me = std::env::current_exe().context("cannot locate this executable")?;
    me.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("{} has no parent folder", me.display()))
}

/// An agent that answered on the pipe.
struct Running {
    client: Client,
    pid: u32,
    /// Handle opened before `Shutdown`, so the pid cannot be reused meanwhile.
    process: Option<Process>,
    image: Option<PathBuf>,
}

/// The agent on `pipe`, if one runs. An agent that answers but refuses this
/// client (an older build with another IPC version) is an error: it cannot be
/// asked to stop, and nothing is changed.
fn find_agent(pipe: &str) -> Result<Option<Running>> {
    match Client::connect_to(pipe) {
        Ok(client) => {
            let pid = client.agent_pid();
            let process = Process::open(pid).ok();
            let image = process.as_ref().and_then(|p| p.image_path().ok());
            Ok(Some(Running {
                client,
                pid,
                process,
                image,
            }))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => bail!(
            "an agent is running but cannot be asked to stop ({e}); it is probably an \
             older build. Stop it (Task Manager: {AGENT_EXE}, or that build's \
             `opensynapse agent stop`) and run this command again. Nothing was changed."
        ),
    }
}

/// Ask the agent to shut down and wait until its process is gone.
fn stop(mut r: Running) -> Result<String> {
    match r.client.call(Request::Shutdown) {
        Ok(Ok(Reply::ShuttingDown)) => {}
        Ok(Ok(other)) => bail!("the agent answered {other:?} to Shutdown"),
        Ok(Err(e)) => bail!("the agent refused to stop: {}", e.message),
        Err(e) => bail!("cannot ask the agent to stop: {e}"),
    }
    drop(r.client);
    match &r.process {
        Some(p) => {
            if p.wait_exit(STOP_WAIT)
                .context("cannot wait for the agent")?
            {
                Ok(format!(
                    "Stopped the running agent ({}).",
                    describe(r.pid, &r.image)
                ))
            } else {
                bail!(
                    "the agent ({}) did not exit within {} s; nothing was copied",
                    describe(r.pid, &r.image),
                    STOP_WAIT.as_secs()
                )
            }
        }
        None => Ok(format!(
            "Asked the running agent (pid {}) to stop; cannot wait for its exit on this OS.",
            r.pid
        )),
    }
}

/// Notes for binaries clearly older than the CLI they are installed with (the
/// first copy is the CLI). Unreadable times give no note.
fn stale_notes(plan: &install::CopyPlan) -> Vec<String> {
    let built = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let Some(cli) = plan.copies.first().and_then(|(from, _)| built(from)) else {
        return Vec::new();
    };
    plan.copies
        .iter()
        .skip(1)
        .filter_map(|(from, _)| {
            let name = from.file_name()?.to_string_lossy();
            install::stale_note(&name, built(from)?, cli)
        })
        .collect()
}

fn describe(pid: u32, image: &Option<PathBuf>) -> String {
    match image {
        Some(i) => format!("pid {pid}, {}", i.display()),
        None => format!("pid {pid}"),
    }
}

/// `opensynapse install [--dir] [--no-autostart]`.
pub fn install(dir: Option<PathBuf>, no_autostart: bool) -> Result<()> {
    // The menu entry (Start menu / app menu) belongs to the default installation only.
    let default_location = dir.is_none();
    let dst = target_dir(dir)?;
    let src = exe_dir()?;
    let plan = install::plan_copy(&src, &dst, Path::is_file).map_err(|e| anyhow!(e))?;
    for note in stale_notes(&plan) {
        println!("{note}");
    }
    let pipe = crate::agent::pipe_name()?;

    let running = find_agent(&pipe)?;
    let was_running = running.is_some();
    if let Some(r) = running {
        println!("{}", stop(r)?);
    }

    std::fs::create_dir_all(&dst).with_context(|| format!("cannot create {}", dst.display()))?;
    for f in install::leftovers(&dst) {
        if std::fs::remove_file(&f).is_ok() {
            println!("Deleted leftover {}", f.display());
        }
    }
    for (from, to) in &plan.copies {
        let how = install::copy_file(from, to)?;
        println!("{} {}", render_copy(&how), to.display());
    }
    for name in &plan.skipped {
        println!("Skipped {name}: not in {}", src.display());
    }

    let gui = dst.join(GUI_EXE);
    if default_location && gui.is_file() && shortcut::launcher_path().is_none() {
        // macOS: no app bundle yet, so no menu entry (ADR-0012).
        println!(
            "GUI: start {} (no menu entry on this system yet).",
            gui.display()
        );
    } else if default_location && gui.is_file() {
        match write_shortcut(&gui, &dst) {
            Ok(p) => println!("{}: {}", shortcut::MENU_NAME, p.display()),
            Err(e) => println!("{}: no shortcut ({e}).", shortcut::MENU_NAME),
        }
    }

    let agent = dst.join(AGENT_EXE);
    let autostart_on = apply_autostart(&agent, &src, no_autostart)?;

    if install::start_after_install(was_running, autostart_on) {
        println!("{}", crate::lifecycle::start_and_wait(&agent, &pipe)?);
    } else {
        println!(
            "Agent not started (none was running, autostart is off). \
             Start it with `opensynapse agent start`."
        );
    }
    println!("Installed to {}.", dst.display());
    Ok(())
}

fn render_copy(how: &Replaced) -> &'static str {
    match how {
        Replaced::New => "Copied",
        Replaced::Replaced => "Updated",
        Replaced::MovedAside => "Updated (old copy still running, renamed to .old)",
    }
}

/// Carry out [`install::plan_autostart`] and print it. Returns whether
/// autostart now starts the installed agent.
fn apply_autostart(agent: &Path, src: &Path, no_autostart: bool) -> Result<bool> {
    let current = match autostart::get() {
        Ok(c) => c,
        Err(e) => {
            println!("Autostart: unchanged ({e}).");
            return Ok(false);
        }
    };
    match install::plan_autostart(current.as_deref(), agent, src, no_autostart) {
        AutostartPlan::Set { previous } => {
            autostart::enable(agent).context("cannot write the autostart entry")?;
            match previous {
                Some(p) => println!(
                    "Autostart: now starts {} at login (was {p}).",
                    autostart::command_line(agent)
                ),
                None => println!(
                    "Autostart: on, starts {} at login.",
                    autostart::command_line(agent)
                ),
            }
            Ok(true)
        }
        AutostartPlan::Unchanged => {
            println!("Autostart: already starts the installed agent.");
            Ok(true)
        }
        AutostartPlan::Leave { current: None } => {
            println!("Autostart: off, left off (--no-autostart).");
            Ok(false)
        }
        AutostartPlan::Leave { current: Some(c) } => {
            println!("Autostart: left as is, starts {c} (--no-autostart).");
            Ok(false)
        }
    }
}

/// Write the GUI's menu entry (Start-menu `.lnk`, Linux `.desktop` launcher)
/// as a temp file + rename.
fn write_shortcut(gui: &Path, dir: &Path) -> io::Result<PathBuf> {
    let path = shortcut::launcher_path()
        .ok_or_else(|| io::Error::other("the menu folder is unknown on this system"))?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let bytes = shortcut::launcher_bytes(
        &gui.to_string_lossy(),
        &dir.to_string_lossy(),
        "OpenSynapse: Razer device settings",
    );
    let tmp = install::with_suffix(&path, install::TMP_SUFFIX);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// `opensynapse uninstall [--dir]`. Deletes only the known binaries (and their
/// `.tmp`/`.old` leftovers) inside the folder, then the folder if it is empty
/// (never the shared Linux `~/.local/bin`). The user's settings stay.
pub fn uninstall(dir: Option<PathBuf>) -> Result<()> {
    let default_location = dir.is_none();
    let dir = target_dir(dir)?;
    let me = std::env::current_exe().context("cannot locate this executable")?;
    if let Some(why) = install::uninstall_refusal(&me, &dir) {
        bail!(why);
    }
    let pipe = crate::agent::pipe_name()?;
    if let Some(r) = find_agent(&pipe)? {
        match &r.image {
            Some(i) if !install::is_in_dir(i, &dir) => println!(
                "Left the running agent alone ({}): not part of this installation.",
                describe(r.pid, &r.image)
            ),
            _ => println!("{}", stop(r)?),
        }
    }

    match autostart::get() {
        Ok(Some(cmd)) if install::points_into(&cmd, &dir) => {
            autostart::disable().context("cannot remove the autostart entry")?;
            println!("Autostart: removed (was {cmd}).");
        }
        Ok(Some(cmd)) => println!("Autostart: left as is, starts {cmd}."),
        Ok(None) => println!("Autostart: off."),
        Err(e) => println!("Autostart: not checked ({e})."),
    }

    if default_location && let Some(lnk) = shortcut::launcher_path() {
        let menu = shortcut::MENU_NAME;
        match std::fs::remove_file(&lnk) {
            Ok(()) => println!("{menu}: removed {}", lnk.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => println!("{menu}: could not remove {} ({e}).", lnk.display()),
        }
    }

    if !dir.is_dir() {
        println!("Nothing installed at {}.", dir.display());
        return Ok(());
    }
    let mut failed = Vec::new();
    for f in install::uninstall_files(&dir) {
        match std::fs::remove_file(&f) {
            Ok(()) => println!("Deleted {}", f.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => failed.push(format!("{}: {e}", f.display())),
        }
    }
    if !failed.is_empty() {
        bail!(
            "could not delete {}\nClose the OpenSynapse GUI if it runs from there and run \
             uninstall again.",
            failed.join("; ")
        );
    }
    if default_location && install::DEFAULT_DIR_SHARED {
        println!("Kept {}: shared with other programs.", dir.display());
    } else {
        match std::fs::remove_dir(&dir) {
            Ok(()) => println!("Deleted folder {}", dir.display()),
            Err(_) => println!("Kept {}: it contains other files.", dir.display()),
        }
    }
    if let Some(c) = config::dir() {
        println!("Your settings in {} were kept.", c.display());
    }
    Ok(())
}
