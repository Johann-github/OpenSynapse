# ADR-0006: Agent autostart = HKCU Run key, GUI subsystem, never elevated

**Status:** accepted (2026-11-11)

**Context:** The agent ([[Architecture/Agent]]) should run after login without a console
window and without admin rights. [[Decisions/ADR-0002 Lean by design]]: no service, no
installer framework, no new dependencies.

**Decision:**
- **Autostart:** one `REG_SZ` value `OpenSynapse Agent` under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, holding the quoted path of
  `opensynapse-agent.exe`. Per user, no admin rights, removable in Task Manager → Startup.
  Managed by `opensynapse agent autostart [on|off]`; the agent binary is the one next to
  the CLI. Rejected: Windows service (runs as SYSTEM in session 0, cannot own the user's
  per-user pipe, needs admin to install), scheduled task (heavier API, admin for some
  triggers).
- **No window:** the agent is a `windows_subsystem = "windows"` binary. Started from a
  terminal it calls `AttachConsole(ATTACH_PARENT_PROCESS)` and opens `CONOUT$` for any
  standard handle the parent did not set, so `-v` logs still show; redirections such as
  `2> agent.log` are kept. Side effect: PowerShell/cmd do not wait for it and
  `$LASTEXITCODE` stays empty; `cargo run` still waits.
- **`opensynapse agent start`:** spawns the agent detached (`DETACHED_PROCESS`, null stdio)
  and waits up to 3 s (50 ms steps, startup only) until the pipe answers.
- **Never elevated:** the agent exits with code 3 if its token is elevated
  (`TokenElevation`). An elevated agent would act with admin rights on requests from
  unelevated clients; the Run key never starts it elevated anyway. `--allow-elevated`
  exists for systems without UAC.
- Code lives in `opensynapse-sys` (registry, token, console FFI via `windows-sys`).

**Consequences:** two new `windows-sys` features (`Win32_System_Registry`,
`Win32_System_Console`), no new crates from crates.io. Linux/macOS autostart (systemd user
unit / LaunchAgent) is still open; there the functions report `Unsupported`.

**Evidence:** hardware test 2026-11-11 (see [[Status]]): `agent start` 141 ms, agent idle
8.4 MB WS / 2.2 MB private, console output visible from Git Bash and PowerShell.
