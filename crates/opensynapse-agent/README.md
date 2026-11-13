# opensynapse-agent

Background agent: owns the HID devices and serves the CLI and GUI over a per-user
named pipe. Decisions: [[Decisions/ADR-0005 Agent IPC]], [[Decisions/ADR-0006 Agent autostart]]
(GUI subsystem, no console window; refuses to run elevated); lean rules:
[[Decisions/ADR-0002 Lean by design]].

| File | Role |
|---|---|
| `main.rs` | args (`--pipe <name>`, `--no-events`, `--app-profiles <file>`, `--allow-elevated`, `-v`), console attach, elevation refusal (exit 3), bind (single instance, exit 2 if running), hotplug, foreground watch, start |
| `core.rs` | core actor on the main thread: the only owner of HID sessions; requests, rescans, events |
| `hardware.rs` | `Hardware` trait (enumerate/open/conflict); `RealHardware`; `FakeHardware` in tests |
| `server.rs` | accept thread, one 64 KiB thread per connection; Control and Subscribe roles |
| `convert.rs` | `opensynapse-device` ↔ `opensynapse-ipc` types |
| `core/apps.rs` | per-app profiles: rules file, switching, restore of the values from before, late apply to hotplugged devices ([[Decisions/ADR-0007 Per-app profiles]]) |
| `foreground.rs`, `foreground_windows.rs` | foreground watch: `SetWinEventHook` thread, posts `Foreground { exe }` on change (FFI only in `foreground_windows.rs`) |
| `foreground_linux.rs`, `foreground_x11.rs` | Linux X11 foreground watch: `PropertyNotify` for `_NET_ACTIVE_WINDOW` → `_NET_WM_PID` → `/proc/<pid>/exe`, one thread in `poll([X socket, eventfd])`; pure helpers (session detection, property decoding, `/proc` link) in `foreground_x11.rs`, unit-tested everywhere. Wayland/headless/macOS → `Unsupported` ([[Decisions/ADR-0013 Foreground watch on Linux and macOS]]) |
| `events.rs` | pluggable `EventSource`; slice 1 ships `NoEvents` ([[Protocol/Hardware Events]]) |

- Writes use `opensynapse_device::guarded_write` (registry check, read old → write →
  read back); refused with `Conflict` while Razer software runs unless confirmed,
  checked at write time.
- Hotplug: `opensynapse_hid::hotplug` (`CM_Register_Notification`); a burst is merged
  into one rescan after a single 150 ms settle. No timers, no polling when idle.
- Per-app profiles: `app-profiles.toml` in the per-user config folder (`%APPDATA%\OpenSynapse`,
  elsewhere `$XDG_CONFIG_HOME/OpenSynapse` or `~/.config/OpenSynapse`); missing = off, invalid = off
  with an error log. Switches use volatile DPI stages and skip-unchanged, never force, and do
  nothing while Razer software runs. `opensynapse agent app-profiles [reload]`.
- Linux exe names: rules see the target of `/proc/<pid>/exe` (e.g. `/usr/lib/firefox/firefox`)
  and match its file name without case, so `exe = "firefox"` works; scripts report their
  interpreter (`python3.12`). X11 sessions only; on Wayland the feature stays off.
- A device that does not answer (`opensynapse_hid::HidError::NoResponse`, e.g. asleep) is
  reported as `ErrorKind::NoResponse` for snapshot and apply; the agent never shows the wake
  hint itself and does not read `settings.toml`. [[Decisions/ADR-0008 User settings and wake hint]]
- Tests run the core against fake mice only; they never touch real hardware.
