# opensynapse-sys

OS integration, kept out of the pure crates. Decision: [[Decisions/ADR-0006 Agent autostart]].

- `autostart`: per-user Run key value `OpenSynapse Agent` (`get`, `enable`, `disable`),
  plus pure helpers `command_line`, `exe_of`, `points_to`.
- `is_elevated`: `TokenElevation` of the current process.
- `attach_parent_console`: for the GUI-subsystem agent; attaches to the parent console
  and opens `CONOUT$` for unset standard handles.
- `agent_exe`: `opensynapse-agent(.exe)` next to the running executable.

Windows only so far (`windows-sys`); elsewhere autostart returns `Unsupported`.
Registry tests use a scratch key `HKCU\Software\OpenSynapse-test-<pid>`, never the Run key.
