# ADR-0009: Fixed per-user install location, `opensynapse install` / `uninstall`

**Status:** accepted (2026-11-11)

**Context:** Autostart ([[Decisions/ADR-0006 Agent autostart]]) and the GUI's "Start agent"
pointed at `<repo>\target\release\opensynapse-agent.exe`. Every `cargo build --release` then
collided with the running agent/GUI: the exe was locked and not replaced, or a stale agent kept
running with an older IPC version (seen 2026-11-11 after IPC v3). [[Decisions/ADR-0002 Lean by design]]:
no installer framework, no MSI, no new dependencies.

**Decision:**
- **Location:** `%LOCALAPPDATA%\Programs\OpenSynapse` (per user, no admin rights; the folder
  Windows uses for per-user programs). Elsewhere `$XDG_DATA_HOME/OpenSynapse/bin` or
  `~/.local/share/OpenSynapse/bin`. One owner: `opensynapse_sys::install::dir`.
- **`opensynapse install [--dir <path>] [--no-autostart]`:** copies `opensynapse.exe`,
  `opensynapse-agent.exe` and (optional) `opensynapse-desktop.exe` from the folder of the running
  CLI. A running agent is asked to stop over IPC; the CLI opens its process handle *before*
  `Shutdown` and blocks on it (bounded 5 s, no polling). Each file goes through `<name>.tmp` +
  rename; a destination that is still running (GUI) is renamed to `<name>.old` and removed by
  the next install/uninstall. Then the installed agent is started detached.
- **Autostart on install:** without `--no-autostart` the Run value is pointed at the installed
  agent. With it, an entry that starts the build being installed from is still moved over (so
  autostart survives a rebuild); any other entry is left alone.
- **`opensynapse uninstall [--dir]`:** stops the agent, removes the Run value only when it points
  into the install folder, deletes only the known binaries and their `.tmp`/`.old` leftovers
  directly in that folder, then the folder if empty. Settings in `%APPDATA%\OpenSynapse` stay.
  Refuses to run from inside the install folder (a running exe cannot delete itself).
- **Which agent:** `opensynapse_sys::agent_exe()` = installed agent if present, else the one next
  to the running exe (dev builds). Used by `agent start`, `agent autostart on`, the GUI's
  "Start agent" and autostart "Fix" (re-resolved on each toggle).
- An agent that answers with another IPC version cannot be asked to stop: install stops with a
  message (stop it in Task Manager) and changes nothing.
- **Start-menu shortcut** (added 2026-11-11): install to the default location writes
  `%APPDATA%\Microsoft\Windows\Start Menu\Programs\OpenSynapse.lnk` for the GUI; uninstall
  (default location) removes it; `--dir` installs never touch the Start menu. No COM: the `.lnk`
  is encoded in `opensynapse_sys::shortcut` from Microsoft's open spec [MS-SHLLINK] (header,
  LinkInfo with ANSI + Unicode local path, StringData name / working dir / icon, an
  EnvironmentVariableDataBlock with the target, terminal block). No LinkTargetIDList.
  Evidence (2026-11-11): a generated shortcut to the installed GUI launched it via
  `Start-Process <lnk>` (observed). With LinkInfo alone and with the env block, WScript's
  `TargetPath` reads empty (it only reports ID-list targets; cosmetic: Properties shows no
  target). Real install with the shortcut: **user step pending** (install touches autostart,
  which Claude may not run).

**Consequences:** rebuilding `target\release` no longer touches the running agent once installed.
The user installs once and re-runs `install` after each build they want to use. Writing the Run
key is a user step (blocked for Claude by the permission settings).
Build with `cargo build --release --workspace`: the workspace's `default-members` is only the CLI,
so a plain `cargo build --release` leaves an old agent and GUI in `target\release`, and install
copies them (2026-11-12: the installed GUI was from 18:49, before the per-app button editor, so the
user saw no Buttons section). `install` now prints a note for a binary more than 2 min older than
the CLI (`install::stale_note`).

**Evidence:** unit tests in `opensynapse-sys` (`install::tests`: paths, copy plan, autostart
plans, uninstall file list, refusal, leftovers; `agent_exe_prefers_installed`). End-to-end
2026-11-11 with `--dir` (scratch folder) and `--no-autostart`: install stopped the running agent
(`target\release`), copied 3 files, started the installed agent (ping OK); a second install over
the running installed agent stopped it, "Updated" all 3, restarted it; uninstall stopped it,
deleted 3 files + folder, kept settings, left the foreign Run entry alone.
Fixed 2026-11-11: piping `install` or `agent start` into another program in Git Bash (`| grep`) hung until
the agent exited (the detached agent inherited the pipe). `spawn_detached` now clears the inherit flag
on the std handles first; `install --dir … | grep -c Copied` returns at once (tested).
