# opensynapse-sys

OS integration, kept out of the pure crates. Decision: [[Decisions/ADR-0006 Agent autostart]].

- `autostart`: per-user Run key value `OpenSynapse Agent` (`get`, `enable`, `disable`),
  plus pure helpers `command_line`, `exe_of`, `points_to`.
- `is_elevated`: `TokenElevation` of the current process.
- `attach_parent_console`: for the GUI-subsystem agent; attaches to the parent console
  and opens `CONOUT$` for unset standard handles.
- `agent_exe`: `opensynapse-agent(.exe)` next to the running executable.
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
