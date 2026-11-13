# ADR-0012: macOS agent = socket in `~/Library/Application Support`, LaunchAgent plist, `~/.local/bin`, `geteuid`

**Status:** accepted (2026-11-13). Stage 5 of [[Architecture/Linux and macOS port]].
Confidence: **inferred**. Pure parts unit-tested on Windows; `opensynapse-sys` and
`opensynapse-ipc` type-checked + clippy-clean (`-D warnings`, `--all-targets`) for
`aarch64-apple-darwin` on the Windows dev PC; not run on a Mac yet (CI `macos-latest` runs the tests).

**Context:** After stage 4 ([[Decisions/ADR-0011 Linux autostart and install]]) the agent had no
socket path on macOS (`XDG_RUNTIME_DIR` is not set there), autostart was `Unsupported`, install
went to a placeholder XDG data folder, and the root check answered "no" because macOS has no
`/proc`. [[Decisions/ADR-0002 Lean by design]]: no new crate, no polling, no extra processes.
The CLI (`agent autostart`, `install`, `uninstall`) and the GUI ("Start agent at login") work
unchanged on top of the same `opensynapse_sys` / `opensynapse_ipc` API.

## IPC socket path
| | Where | For | Against |
|---|---|---|---|
| a | `$TMPDIR` (per-user `/var/folders/…/T`, 0700) | private, short path | cleaned by the system (at boot and of old items), which could delete a live socket; a value from the environment must match between a launchd-started agent and a Terminal client |
| b | `~/Library/Caches/OpenSynapse` | private (`~/Library` is 0700) | caches may be purged under storage pressure |
| c | **`~/Library/Application Support/OpenSynapse/opensynapse-agent.sock`** | private, never cleaned, needs only `HOME` (launchd sets it for agents) | longer path; contains a space (harmless for sockets) |

**Decision: (c).** `XDG_RUNTIME_DIR` is ignored on macOS even when a user exports it: it would
reach a Terminal client but not the launchd-started agent, and both must agree. `HOME` must be
absolute. `Listener::bind` creates the `OpenSynapse` folder with mode 0700 when it is missing
(an existing folder is left as it is); the socket itself stays 0600 as on Linux. `sun_path` holds
104 bytes on macOS (108 on Linux), NUL included: a path over 103 bytes (home folder name over
~44 characters) is refused with a clear error instead of a failing bind (open: a short fallback).
Linux keeps `$XDG_RUNTIME_DIR/opensynapse-agent.sock`; only a blank value is now refused (it
used to give a relative path). Pure choice: `opensynapse_ipc::transport::socket_path`.

## Autostart
| | What | For | Against |
|---|---|---|---|
| a | **LaunchAgent plist `~/Library/LaunchAgents/io.github.johann-github.opensynapse.agent.plist`** | the per-user launchd mechanism; one text file; no admin | macOS 13+ shows a "background item added" notice; the user can switch it off in Login Items, which lives in a system database we cannot read |
| b | Login Item through `SMAppService` | the modern API | needs an app bundle, Objective-C/Swift bridging and code signing |

**Decision: (a).** Keys: `Label` (the reverse-DNS label above, file = `<label>.plist`),
`ProgramArguments` = `[<agent path>]`, `RunAtLoad` true, `KeepAlive` false (no restart: the exit
codes 2 "already running" and 3 "root" must not loop), `LimitLoadToSessionType` `Aqua` (graphical
login only, like the XDG autostart entry). The path is XML-escaped (`& < > " '`); control
characters are refused. Written as temp file + rename (`config::write`); `agent autostart off`
deletes the file; a missing file is "off". Reading back (`launch_agent::program`): first
`ProgramArguments` string (or `Program`), off when `Disabled` is true or `RunAtLoad` is not true.
The "command" handed around on macOS is the plain agent path; `points_to` compares exactly (a
case-only mismatch on a case-insensitive volume just rewrites our plist).

**No `launchctl`.** launchd reads `~/Library/LaunchAgents` at the next login, which is all
"start at login" promises. Starting the agent now is already done by `install` / `agent start`
through `spawn_detached`, so `launchctl bootstrap gui/<uid>` would only add a subprocess and a
second agent instance racing for the socket (exit 2). Consequence: after `agent autostart off`
a job loaded at this login stays known to launchd until logout (the running agent is not killed;
it is not restarted either, `KeepAlive` false).

## Install location
**`~/.local/bin`**, same as Linux (`install::dir`, `DEFAULT_DIR_SHARED` true, so uninstall keeps
the folder). It is the folder per-user CLI installers commonly use on macOS too; not on the
default zsh `PATH`, but one `PATH` line away, and the same instructions as on Linux. Rejected:
`~/Applications/OpenSynapse/` (bare binaries in a Finder apps folder; not on `PATH` either).
**GUI launcher:** none (`shortcut::launcher_path` is `None` on macOS; `install` prints the GUI
path instead). The Start-menu equivalent is an `.app` bundle in `~/Applications` (Info.plist +
binary, ideally signed) — **open**, out of scope here.

## Root refusal
The agent exits with code 3 when its effective uid is 0, as on Linux. macOS has no `/proc`, so
`opensynapse-sys` declares libSystem's `geteuid` itself (`unsafe extern "C" { safe fn geteuid()
-> u32; }`, `uid_t` is 32-bit unsigned on Darwin) in `src/macos.rs`, the crate's only macOS
`unsafe` (allowed per module, like `windows.rs`). Rejected: the `libc` crate (in the lock file
through hidapi, but a new direct dependency of this crate for one call).

## Input reader (not changed)
The macOS input reader still wakes once per second (`hidapi_input.rs`, `read_timeout(1000)`).
A blocking version needs `IOHIDDeviceRegisterInputReportCallback` on a CFRunLoop stopped with
`CFRunLoopStop` from `stop()` — hand-written IOKit/CoreFoundation FFI with callback lifetimes
that cannot be checked without a Mac. **Open** until a Mac session.

**Consequences:** no new dependency, no subprocess, no polling added. Untested on macOS
hardware; `Process::open` (waiting for the old agent during `install`) is still Windows-only,
as on Linux.

**Evidence:** unit tests `opensynapse-ipc` `transport::socket_path::tests` (both layouts, blank
and relative values, the 103/107-byte limits), `opensynapse-sys` `launch_agent::tests` (path
from `HOME`, rendered keys, XML escape round trip incl. `& < > " '` and `ü`, control characters
refused, `Disabled` / `RunAtLoad` false, comments, foreign keys, `Program`), macOS-only
`macos_command_is_the_plain_path` (CI). `cargo clippy -p opensynapse-sys -p opensynapse-ipc
--target aarch64-apple-darwin --all-targets -- -D warnings` clean on 2026-11-13 (crates that
build hidapi's C code need the Apple SDK and are checked by CI only). Plist keys and their
meaning from Apple's `launchd.plist(5)`; `sun_path` size from Darwin `<sys/un.h>` (inferred, no
Mac tested).
