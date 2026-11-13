# opensynapse-sys

OS integration, kept out of the pure crates. Decisions: [[Decisions/ADR-0006 Agent autostart]],
[[Decisions/ADR-0009 Install location]], [[Decisions/ADR-0011 Linux autostart and install]],
[[Decisions/ADR-0012 macOS agent]].

- `autostart`: per-user Run key value `OpenSynapse Agent` on Windows; on Linux the XDG
  autostart entry `$XDG_CONFIG_HOME/autostart/opensynapse-agent.desktop`; on macOS the
  LaunchAgent `~/Library/LaunchAgents/io.github.johann-github.opensynapse.agent.plist`
  (`get`, `enable`, `disable`), plus helpers `command_line`, `exe_of`, `points_to` (Run value on
  Windows, `Exec=` value on Linux, plain path on macOS) and the pure Windows forms
  `windows_command_line` / `windows_exe_of`.
- `launch_agent`: pure LaunchAgent plist code for macOS (runs and is tested on every OS):
  render (`RunAtLoad` true, `KeepAlive` false, `LimitLoadToSessionType` Aqua), XML escape /
  unescape, reading the program back (off when `Disabled` or no `RunAtLoad`), path from `HOME`.
  No `launchctl`; launchd picks the file up at the next login.
- `desktop_entry`: pure Desktop Entry code for Linux (runs and is tested on every OS): `Exec=`
  quoting and parsing, string escapes, autostart entry and GUI launcher text, XDG folder
  resolution from given env values, effective uid from `/proc/self/status` text.
- `is_elevated`: `TokenElevation` of the current process; Linux: effective uid 0 (root), read
  from `/proc/self/status` with std (no `libc`); macOS: `geteuid` declared by hand in
  `src/macos.rs` (no `libc` crate). `ELEVATED_NAME` = `administrator` / `root`.
- `attach_parent_console`: for the GUI-subsystem agent; attaches to the parent console
  and opens `CONOUT$` for unset standard handles.
- `agent_exe`: the installed agent (`install::dir`) if present, else `opensynapse-agent(.exe)`
  next to the running executable (pure choice: `resolve_agent_exe`).
- `install`: per-user install folder `%LOCALAPPDATA%\Programs\OpenSynapse` (Linux `~/.local/bin`,
  and macOS `~/.local/bin`, shared, so uninstall keeps it: `DEFAULT_DIR_SHARED`) and the
  pure plan of `opensynapse install` / `uninstall` (copy plan, autostart plan, files to delete,
  leftovers), plus `copy_file` (temp + rename, running exe renamed to `.old`).
- `shortcut`: pure `.lnk` encoder ([MS-SHLLINK], no COM) and the Start-menu folder;
  `launcher_path` / `launcher_bytes` pick the GUI menu entry for this OS (`.lnk` on Windows,
  `$XDG_DATA_HOME/applications/opensynapse.desktop` on Linux); used by `install`.
- `udev`: the Linux udev rule `data/udev/70-opensynapse.rules` (`UDEV_RULE`, embedded with
  `include_str!`, one owner) plus `install_hint`. Plain text on every OS; printed by
  `opensynapse udev-rule`. [[Architecture/Linux and macOS port]]
- `Process`: open a process by pid, read its image path, block until it exits (no polling).
- `spawn_detached`: the generic detached spawn behind `spawn_agent_detached`.
- `spawn_agent_detached`: starts `agent_exe` with `DETACHED_PROCESS |
  CREATE_NEW_PROCESS_GROUP` and null stdio, without waiting. Shared by the CLI
  (`agent start`, which then waits for the pipe) and the GUI ("Start agent").
- `config`: per-user config folder `%APPDATA%\OpenSynapse` (`$XDG_CONFIG_HOME` or
  `~/.config` elsewhere), file names `settings.toml` / `app-profiles.toml`,
  `settings_path`, `read_optional` (missing file = `None`) and `write` (temp file + rename).
  Shared by the agent, CLI and GUI. [[Decisions/ADR-0008 User settings and wake hint]]
- `agent_exit`: the agent's exit codes callers react to (`ALREADY_RUNNING` = 2,
  `ELEVATED` = 3).

Windows (`windows-sys`), Linux (std only) and macOS (std + one hand-declared libSystem call).
`shortcut::launcher_path` is `None` on macOS (no `.app` bundle yet; `install` prints the GUI path).
Registry tests use a scratch key `HKCU\Software\OpenSynapse-test-<pid>`, never the Run key;
Desktop Entry tests are pure (no file is written to the real autostart or menu folders).
Linux type-check: `cargo clippy -p opensynapse-sys --target x86_64-unknown-linux-gnu --all-targets`;
macOS: same with `--target aarch64-apple-darwin` (no C build script, works without the Apple SDK).
