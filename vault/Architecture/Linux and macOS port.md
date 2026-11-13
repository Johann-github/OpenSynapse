---
tags: [plan, platform]
updated: 2026-11-13
---
# Linux and macOS port (plan)

Second platform goal from [[Home]] / [[Backlog]]. Decided with the user 2026-11-13:
**CI first, hardware later.** No Linux/macOS machine and no WSL on the dev PC, so every step
below is checked by GitHub Actions until a hardware test is possible.

## Inventory (2026-11-13, from the code)
| Area | Windows | Linux / macOS today |
|---|---|---|
| HID feature reports | own SetupAPI backend ([[Decisions/ADR-0003 Own Windows HID backend]]) | `hidapi_backend.rs`, never compiled |
| HID input reports (events) | `windows_input.rs` | `hidapi_input.rs` (stage 3 part 2, untested on hardware) |
| Hotplug | `windows_hotplug.rs` | `Unsupported` |
| IPC | named pipe | `transport/unix.rs`, socket in `$XDG_RUNTIME_DIR` (compile-only; macOS has no `XDG_RUNTIME_DIR`) |
| Autostart | HKCU Run ([[Decisions/ADR-0006 Agent autostart]]) | `Unsupported` |
| Install location | `%LOCALAPPDATA%\Programs` ([[Decisions/ADR-0009 Install location]]) | none |
| Per-app foreground watch | WinEvent hook ([[Decisions/ADR-0007 Per-app profiles]]) | `Unsupported` (feature inactive) |
| GUI | eframe/glow | eframe has `x11` + `wayland` features for Linux |

## Stages
1. ✅ **CI** (2026-11-13): GitHub Actions on `windows-latest`, `ubuntu-latest`, `macos-latest`:
   `fmt --check`, `clippy -D warnings`, `test --workspace`. Linux needs `libudev-dev`
   (hidapi hidraw) and `libxkbcommon-dev` (eframe). Runs on every branch push.
2. ✅ **Compile clean** (2026-11-13, run 43450884428 green on all three): everything already
   compiled off Windows. Fixed only lints (`drop_non_drop` on the agent's shutdown drops,
   `dead_code` on `Dedup`) and two tests with Windows path assumptions in `opensynapse-sys`
   (`Path::join` separator; `starts_with` on `C:\…`, which is one component on Unix).
3. **Linux HID**: hidapi hidraw backend; udev rule shipped in `data/` (`TAG+="uaccess"` for VID
   1532) so no root is needed; input reports via a reader thread (blocking read, no polling);
   hotplug without polling (udev netlink monitor or inotify on `/dev`, needs an ADR + dependency
   check against [[Decisions/ADR-0002 Lean by design]]).
   - Part 2, input reports (2026-11-13, **inferred**, compile/clippy-checked for Linux
     locally, untested on hardware): `hidapi_input.rs`, one reader thread per collection with
     hidapi `read_timeout(1000 ms)`. hidapi cannot unblock a read from another thread
     (`HidDevice` is not `Sync`; closing under a blocked read is unsafe), so the thread wakes
     once per second without a report to check a stop flag: no device I/O, but not the
     zero-wakeup idle of Windows; `stop` takes ≤ 1 s. Byte 0 = report ID like Windows
     (unnumbered reports get a 0 in front, decided from the report descriptor). Buffer 1 KiB;
     hidapi gives no input report length (`input_len` = `None`). Enumeration now keeps
     collections that share a hidraw path (dedup key includes usage page/usage). macOS opens
     non-exclusively (`set_open_exclusive(false)`, hidapi's default is exclusive).
     Open: on Linux the hidraw node carries all collections of the interface, so readers see
     every report of it (agent filters on byte 0); fully blocking reader via raw fd + `poll`
     only if the 1 s wakeup ever matters.
4. **Linux agent**: autostart as an XDG autostart `.desktop` file or a systemd user unit (ADR),
   install to `~/.local/bin`, socket as today.
5. **macOS**: hidapi IOKit (check: non-exclusive open, so the OS keeps the mouse), socket path
   fallback when `XDG_RUNTIME_DIR` is unset (e.g. `$TMPDIR`), LaunchAgent plist for autostart.
6. **Per-app profiles off Windows** (optional): X11 `_NET_ACTIVE_WINDOW`; Wayland has no
   generic API (stay `Unsupported`); macOS `NSWorkspace` activation notifications.
7. **Hardware test** on a real Linux/macOS machine with the Viper (or WSL2 + usbipd): same
   checklist as phase 3 ([[RE-Log/2026-11-09 Hardware read test]]).

## Open questions
- Does hidapi on Linux report usage page/usage per collection for the Viper (needed for the
  control-collection match, `CONTROL_USAGE_PAGE`/`CONTROL_USAGE`)? **unknown** until hardware.
- macOS: are feature reports allowed without the "Input Monitoring" permission? **unknown**.
