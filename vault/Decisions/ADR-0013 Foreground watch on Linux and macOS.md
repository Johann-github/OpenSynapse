# ADR-0013: Foreground watch off Windows = X11 `_NET_ACTIVE_WINDOW` via `x11rb`; Wayland off; macOS pending

**Status:** accepted (2026-11-13). Stage 6 of [[Architecture/Linux and macOS port]].
Confidence: **inferred**. Linux code is type-checked and clippy-clean for
`x86_64-unknown-linux-gnu` and checked by CI; it has not run in a real X11 session yet.

**Context:** Per-app profiles ([[Decisions/ADR-0007 Per-app profiles]]) need one callback per
change of the foreground executable: `foreground::watch(callback) -> Watch`, drop stops it, the
callback gets the executable's full path, deduplicated (`Dedup`). Rules match by file name
without case, or by full path when the pattern has a separator
(`opensynapse_core::app_profiles::exe_matches`). [[Decisions/ADR-0002 Lean by design]]: event
driven only, no polling, idle CPU ≈ 0 %, every crate justified.

## Linux X11
**Mechanism (EWMH):** the window manager keeps the active window id in the root window property
`_NET_ACTIVE_WINDOW`. The agent selects `PropertyChangeMask` on the root (event masks are per
client, so this changes nothing for the WM) and gets a `PropertyNotify` whenever it changes. The
active window's `_NET_WM_PID` (CARDINAL) gives the process, and `readlink /proc/<pid>/exe` the
executable. The value is read once at start too, like on Windows.

**Library options:**
| | What | Runtime need | Cargo.lock | Notes |
|---|---|---|---|---|
| a | `x11rb` 0.13, no features: `RustConnection` speaks the X protocol over the socket itself | none (no libX11/libxcb) | already locked (eframe → winit); deps `x11rb-protocol`, `rustix`, `gethostname`, all locked | X errors (e.g. `BadWindow` for a window that just closed) come back as `Result`; socket fd for `poll` via `stream().as_raw_fd()`; handles `DISPLAY` + `XAUTHORITY` |
| b | `x11-dl`: dlopen libX11 at runtime | libX11 (else clean error) | already locked | Xlib's default error handler **exits the process** on `BadWindow`; we would have to install a process-global `XSetErrorHandler`, which also affects any other Xlib user. More unsafe FFI |
| c | own X11 wire protocol + Xauthority parser | none | none | ~400 lines of protocol code nobody can test here |

**Decision:** (a). No new package in `Cargo.lock` (only two dependency lines for the agent),
pure Rust, no C library at build or run time, and X errors cannot kill the agent. `libc`
(already direct in `opensynapse-hid` for Linux) for `eventfd`/`poll`.

**Thread** (`crates/opensynapse-agent/src/foreground_linux.rs`): connect, intern
`_NET_ACTIVE_WINDOW` + `_NET_WM_PID`, select on the root and check the request on the caller's
thread (errors return from `watch` at once), then one thread `foreground` (256 KiB stack):
1. drain `poll_for_event()` first (waiting for a reply also reads events into x11rb's buffer,
   which `poll` would never announce again);
2. any `PropertyNotify(root, _NET_ACTIVE_WINDOW)` in the batch → one lookup → `Dedup` →
   callback; then drain again;
3. otherwise sleep in `poll([X socket, eventfd], -1)`.
Stop: `Drop` writes the eventfd and joins (as [[Decisions/ADR-0010 Linux hotplug]]); a running
callback finishes first. Connection loss (`POLLHUP`/`POLLERR`, read error, X server gone at
session end) logs a warning and ends the thread; per-app switching stops until the agent
restarts.

**Pure parts** (`foreground_x11.rs`, compiled and unit-tested on every platform): session
detection, `card32(format, bytes)` (first item, format 32, 0 = none), `/proc/<pid>/exe`,
link target → exe (drops the kernel's ` (deleted)` suffix after a package update, rejects
non-paths).

**Exe names on Linux:** the callback gets the real binary, e.g. `/usr/lib/firefox/firefox`,
`/usr/share/code/code`. The core matches the file name without case, so a rule
`exe = "firefox"` works, `"firefox.exe"` does not, and a full path works as a full path.
Scripts and launchers that `exec` an interpreter report the interpreter (`python3.12`,
`java`); a rule then has to name that. Not changed in the core: one contract for every
platform.

## Linux Wayland
**Decision: `Unsupported`.** Wayland has no generic, unprivileged "which window is active"
API (only compositor-specific protocols such as wlr-foreign-toplevel or KDE/GNOME scripting,
and those give an app id, not a pid). The X11 path is **not** used when only XWayland is
present: XWayland sees only X11 clients, so native Wayland apps in front would look like
"nothing changed" and keep the last X11 app's profile, which is worse than off. Detection:
`WAYLAND_DISPLAY` set or `XDG_SESSION_TYPE=wayland` → Wayland, even if `DISPLAY` is set;
else `DISPLAY` set → X11; else headless → `Unsupported`. The agent logs the error as today
("no foreground notifications; per-app profiles inactive").

## macOS
**Decided, pending (not implemented):** observe `NSWorkspaceDidActivateApplicationNotification`
on `NSWorkspace.sharedWorkspace.notificationCenter`; the notification's
`NSWorkspaceApplicationKey` → `NSRunningApplication.executableURL.path` is the exe (for an app
bundle the binary inside `Contents/MacOS/`, e.g. `…/Safari.app/Contents/MacOS/Safari`, so a
rule `Safari` matches). Bindings: `objc2` + `objc2-foundation` + `objc2-app-kit` + `block2`
(all already locked through eframe/winit; feature-gated, no hand-written `objc_msgSend`).

Why not now: whether the workspace notifications reach an observer in a background-only
process whose **main thread does not run a run loop** (our main thread is the core actor) is
**unknown**. Reports differ between "delivered on the posting (main) thread, needs the main
run loop" and "a run loop on any thread is enough". Stopping a `CFRunLoopRun` on the watch
thread (`CFRunLoopStop` from `Drop`) and the block lifetime are straightforward, but the
delivery question can only be answered on a Mac, and a watch that starts but never fires would
fail silently. So macOS keeps `Unsupported` until it can be tested (stage 7), then this ADR
gets the implementation.

## Also changed
The agent's default rules file now comes from `opensynapse_sys::config::dir()`
(`%APPDATA%\OpenSynapse` on Windows, unchanged; `$XDG_CONFIG_HOME/OpenSynapse` or
`~/.config/OpenSynapse` elsewhere). Before, it read `APPDATA` only, so off Windows the feature
had no default file. The GUI's fallback when no agent runs still reads `APPDATA` (open).

## Not done / accepted risks
- `_NET_WM_PID` is set by the client. Missing (some toolkits, old apps) → that window is
  ignored and the previous profile stays, like an unqueryable process on Windows.
- Remote X clients (ssh -X) report a pid of the remote host; `/proc/<pid>/exe` then names an
  unrelated local process or nothing. Not checked against `WM_CLIENT_MACHINE`. Rare.
- Sandboxed apps in their own pid namespace (Flatpak) may report the sandbox pid → wrong or no
  exe. Possible later fix: the X-Resource extension (`XResQueryClientIds`, server-side pid).
- A WM without EWMH never sets `_NET_ACTIVE_WINDOW` → the watch runs but never fires.
- Processes of other users (`/proc/<pid>/exe` is EACCES) are ignored.
- Binary-size impact of `x11rb` on the Linux agent is not measured yet (no Linux linker here).

**Consequences:** On X11, one sleeping thread and one X connection; one wake-up per focus
change. Hardware/session check (stage 7): run the agent with `-v` in an X11 session, rule for
`gnome-terminal-server` or `xterm`, switch windows, expect `Foreground` lines and switches.
