# opensynapse-sys

OS integration, kept out of the pure crates. Decisions: [[Decisions/ADR-0006 Agent autostart]],
[[Decisions/ADR-0009 Install location]].

- `autostart`: per-user Run key value `OpenSynapse Agent` (`get`, `enable`, `disable`),
  plus pure helpers `command_line`, `exe_of`, `points_to`.
- `is_elevated`: `TokenElevation` of the current process.
- `attach_parent_console`: for the GUI-subsystem agent; attaches to the parent console
  and opens `CONOUT$` for unset standard handles.
- `agent_exe`: the installed agent (`install::dir`) if present, else `opensynapse-agent(.exe)`
  next to the running executable (pure choice: `resolve_agent_exe`).
- `install`: per-user install folder `%LOCALAPPDATA%\Programs\OpenSynapse` (XDG elsewhere) and the
  pure plan of `opensynapse install` / `uninstall` (copy plan, autostart plan, files to delete,
  leftovers), plus `copy_file` (temp + rename, running exe renamed to `.old`).
- `shortcut`: pure `.lnk` encoder ([MS-SHLLINK], no COM) and the Start-menu folder; used by
  `install` for the GUI shortcut.
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

Windows only so far (`windows-sys`); elsewhere autostart returns `Unsupported`.
Registry tests use a scratch key `HKCU\Software\OpenSynapse-test-<pid>`, never the Run key.
