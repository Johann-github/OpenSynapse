# opensynapse-agent

Background agent: owns the HID devices and serves the CLI and GUI over a per-user
named pipe. Decision: [[Decisions/ADR-0005 Agent IPC]]; lean rules:
[[Decisions/ADR-0002 Lean by design]].

| File | Role |
|---|---|
| `main.rs` | args (`--pipe <name>`, `--no-events`, `--app-profiles <file>`, `-v`), bind (single instance, exit 2 if running), hotplug, foreground watch, start |
| `core.rs` | core actor on the main thread: the only owner of HID sessions; requests, rescans, events |
| `hardware.rs` | `Hardware` trait (enumerate/open/conflict); `RealHardware`; `FakeHardware` in tests |
| `server.rs` | accept thread, one 64 KiB thread per connection; Control and Subscribe roles |
| `convert.rs` | `opensynapse-device` ↔ `opensynapse-ipc` types |
| `core/apps.rs` | per-app profiles: rules file, switching, restore of the values from before ([[Decisions/ADR-0007 Per-app profiles]]) |
| `foreground.rs`, `foreground_windows.rs` | foreground watch: `SetWinEventHook` thread, posts `Foreground { exe }` on change (FFI only in `foreground_windows.rs`) |
| `events.rs` | pluggable `EventSource`; slice 1 ships `NoEvents` ([[Protocol/Hardware Events]]) |

- Writes use `opensynapse_device::guarded_write` (registry check, read old → write →
  read back); refused with `Conflict` while Razer software runs unless confirmed,
  checked at write time.
- Hotplug: `opensynapse_hid::hotplug` (`CM_Register_Notification`); a burst is merged
  into one rescan after a single 150 ms settle. No timers, no polling when idle.
- Per-app profiles: `%APPDATA%\OpenSynapse\app-profiles.toml`; missing = off, invalid = off
  with an error log. Switches use volatile DPI stages and skip-unchanged, never force, and do
  nothing while Razer software runs. `opensynapse agent app-profiles [reload]`.
- Tests run the core against fake mice only; they never touch real hardware.
