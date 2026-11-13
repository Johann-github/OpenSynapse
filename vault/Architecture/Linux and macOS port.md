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
| HID feature reports | own SetupAPI backend ([[Decisions/ADR-0003 Own Windows HID backend]]) | `hidapi_backend.rs` (CI-compiled, untested on hardware) |
| Hotplug | `windows_hotplug.rs` | Linux: udev netlink ([[Decisions/ADR-0010 Linux hotplug]]); macOS `Unsupported` |
| HID input reports (events) | `windows_input.rs` | Linux `linux_input.rs` (hidraw + `poll`/eventfd); macOS `hidapi_input.rs` (stage 3 part 2, untested on hardware) |
| IPC | named pipe | `transport/unix.rs`: Linux socket in `$XDG_RUNTIME_DIR`; macOS `~/Library/Application Support/OpenSynapse/opensynapse-agent.sock` ([[Decisions/ADR-0012 macOS agent]], inferred) |
| Autostart | HKCU Run ([[Decisions/ADR-0006 Agent autostart]]) | Linux: XDG autostart entry `~/.config/autostart/opensynapse-agent.desktop` ([[Decisions/ADR-0011 Linux autostart and install]], inferred); macOS: LaunchAgent `~/Library/LaunchAgents/io.github.johann-github.opensynapse.agent.plist` (ADR-0012, inferred) |
| Install location | `%LOCALAPPDATA%\Programs` ([[Decisions/ADR-0009 Install location]]) | Linux: `~/.local/bin` + app-menu launcher `~/.local/share/applications/opensynapse.desktop` (ADR-0011, inferred); macOS: `~/.local/bin`, no GUI launcher yet (`.app` bundle open; ADR-0012, inferred) |
| Refuse elevated agent | `TokenElevation` | Linux euid from `/proc/self/status`; macOS `geteuid` (own declaration, no `libc`; ADR-0012) |
| Per-app foreground watch | WinEvent hook ([[Decisions/ADR-0007 Per-app profiles]]) | Linux X11: `_NET_ACTIVE_WINDOW` `PropertyNotify` via `x11rb` ([[Decisions/ADR-0013 Foreground watch on Linux and macOS]], inferred); Wayland/headless and macOS `Unsupported` (macOS decided, pending) |
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
   - Part 1 ✅ **udev rule** (2026-11-13): `data/udev/70-opensynapse.rules`, printed by
     `opensynapse udev-rule` (owner: `opensynapse_sys::udev::UDEV_RULE`). See
     [[#Linux setup]]. Not hardware-tested.
   - Part 3 ✅ **hotplug** (2026-11-13): netlink `NETLINK_KOBJECT_UEVENT`, udev group 2, one thread in
     `poll` + eventfd stop; `libc` (already in the tree through hidapi), so no new crate →
     [[Decisions/ADR-0010 Linux hotplug]]. Parser unit-tested on Windows, Linux code
     type-checked + clippy-clean locally (`--target x86_64-unknown-linux-gnu`), CI pending.
     Confidence **inferred** (no hardware test yet).
   - Part 2 ✅ **input reports** (2026-11-13, **inferred**, compile/clippy-checked for Linux
     locally, untested on hardware). One reader thread per collection; byte 0 = report ID like
     Windows (unnumbered reports get a 0 in front, decided from the report descriptor); buffer
     1 KiB, no input report length known (`input_len` = `None`).
     **Linux: blocking `poll` + eventfd, no wakeups** (`linux_input.rs`, 2026-11-13): the hidraw
     node (`/dev/hidrawN`, the path hidapi enumerates) opened read-only with `std::fs`, the
     thread sleeps in `poll([hidraw, eventfd], -1)` and `read`s one report per POLLIN; `stop`/Drop
     write the eventfd and join, so `stop` returns at once. POLLHUP/POLLERR or a read error
     (ENODEV/EIO, device unplugged) ends the reader with an error, like the hidapi read error
     did. Descriptor from sysfs `/sys/class/hidraw/hidrawN/device/report_descriptor` (same
     bytes as `HIDIOCGRDESC`); unreadable → assume numbered. Same pattern as
     [[Decisions/ADR-0010 Linux hotplug]], no new dependency (`libc`).
     **macOS: still 1 s wakeup** (open item): `hidapi_input.rs`, hidapi `read_timeout(1000 ms)`.
     hidapi cannot unblock a read from another thread (`HidDevice` is not `Sync`; closing under
     a blocked read is unsafe), so the thread wakes once per second without a report to check a
     stop flag: no device I/O, but not zero-wakeup idle; `stop` takes ≤ 1 s. A blocking version
     would need an IOHIDDevice input callback on a CFRunLoop outside hidapi.
     Enumeration keeps collections that share a hidraw path (dedup key includes usage
     page/usage). macOS opens non-exclusively (`set_open_exclusive(false)`, hidapi's default is
     exclusive). Open: on Linux the hidraw node carries all collections of the interface, so
     readers see every report of it (agent filters on byte 0).
4. ✅ **Linux agent** (2026-11-13, **inferred**, unit-tested on Windows, `opensynapse-sys`
   type-checked + clippy-clean for `x86_64-unknown-linux-gnu`, no Linux session tested):
   autostart = XDG autostart entry `$XDG_CONFIG_HOME/autostart/opensynapse-agent.desktop`
   (not a systemd user unit: works in every desktop session and starts inside the graphical
   session, where the `uaccess` ACL and a later X11 foreground watch apply); install to
   `~/.local/bin` (on `PATH`; uninstall keeps the shared folder) + GUI launcher
   `$XDG_DATA_HOME/applications/opensynapse.desktop`; agent refuses euid 0 (exit 3, from
   `/proc/self/status`, std only). Pure Desktop Entry code in `opensynapse_sys::desktop_entry`
   (`Exec=` quoting per spec). Same CLI/GUI API. Socket as today. →
   [[Decisions/ADR-0011 Linux autostart and install]]. Open: `Process::open` (wait for the old
   agent's exit during `install`) is still Windows-only.
5. ✅ **macOS agent** (2026-11-13, **inferred**, unit-tested on Windows, `opensynapse-sys` +
   `opensynapse-ipc` type-checked + clippy-clean for `aarch64-apple-darwin`, no Mac tested):
   socket `~/Library/Application Support/OpenSynapse/opensynapse-agent.sock` (folder created
   0700, needs only `HOME`; `XDG_RUNTIME_DIR` ignored on macOS; `$TMPDIR` rejected because the
   system cleans it; paths over 103 bytes refused); autostart = LaunchAgent plist in
   `~/Library/LaunchAgents` (`RunAtLoad` true, `KeepAlive` false, `LimitLoadToSessionType` Aqua,
   XML-escaped path, no `launchctl`: takes effect at next login); install to `~/.local/bin`
   (shared, like Linux); agent refuses euid 0 via a hand-declared `geteuid` (no `libc` crate).
   → [[Decisions/ADR-0012 macOS agent]]. Open: GUI launcher (`.app` bundle in `~/Applications`);
   input reader still wakes once per second (blocking IOKit callback + CFRunLoop needs a Mac to
   test); hidapi non-exclusive open and Login Items switch-off unverified; `Process::open`
   still Windows-only.
6. **Per-app profiles off Windows** (optional): X11 `_NET_ACTIVE_WINDOW`; Wayland has no
   generic API (stay `Unsupported`); macOS `NSWorkspace` activation notifications.
   - ✅ **Linux X11** (2026-11-13, **inferred**, pure parts unit-tested on Windows, Linux code
     type-checked + clippy-clean for `x86_64-unknown-linux-gnu`, no X11 session tested):
     `x11rb` (pure-Rust connection, no features, already locked through eframe/winit, no new
     package) selects `PropertyChangeMask` on the root; `PropertyNotify(_NET_ACTIVE_WINDOW)` →
     `_NET_WM_PID` → `/proc/<pid>/exe`. One thread in `poll([X socket, eventfd], -1)`, drop
     stops + joins. Wayland (`WAYLAND_DISPLAY` / `XDG_SESSION_TYPE=wayland`, even with
     XWayland) and headless → `Unsupported`. Default rules file off Windows now
     `$XDG_CONFIG_HOME/OpenSynapse/app-profiles.toml`. →
     [[Decisions/ADR-0013 Foreground watch on Linux and macOS]].
   - **macOS: decided, pending** (objc2 + `NSWorkspaceDidActivateApplicationNotification`):
     whether the notification reaches a non-main-thread run loop is unknown without a Mac.
7. **Hardware test** on a real Linux/macOS machine with the Viper (or WSL2 + usbipd): same
   checklist as phase 3 ([[RE-Log/2026-11-09 Hardware read test]]).

## Linux setup
hidraw nodes (`/dev/hidraw*`) are root-only by default. OpenSynapse ships a udev rule instead
of asking for root or a world-writable mode:

    opensynapse udev-rule > 70-opensynapse.rules
    sudo cp 70-opensynapse.rules /etc/udev/rules.d/
    sudo udevadm control --reload-rules && sudo udevadm trigger

then replug the device or receiver. The command only prints (rule on stdout, steps on stderr).

| Claim | Confidence | Evidence |
|---|---|---|
| `TAG+="uaccess"` makes systemd-logind put an ACL for the active seat user on the node | inferred | systemd `uaccess` convention (applied by `73-seat-late.rules`, so our file must sort before 73) |
| USB and receiver: a parent USB device has `idVendor` 1532, matched with `ATTRS{idVendor}` | inferred | standard udev parent walk; Razer VID from [[Devices/Viper V3 HyperSpeed]] / `data/devices/` |
| Bluetooth: no USB parent; the HID parent is named `0005:1532:<PID>.<n>` (HID_ID `0005:00001532:…`), matched with `KERNELS=="0005:1532:*"` | inferred | kernel HID device naming (`bus:vendor:product.instance`, 4 hex digits) |
| No `MODE=` / `0666`: other users and system services get no access | observed | rule text, unit test `rule_sets_no_open_mode` |

Untested on hardware. Systems without systemd-logind (no `uaccess`) need a group-based rule
instead; not covered yet.

## Open questions
- Does hidapi on Linux report usage page/usage per collection for the Viper (needed for the
  control-collection match, `CONTROL_USAGE_PAGE`/`CONTROL_USAGE`)? **unknown** until hardware.
- macOS: are feature reports allowed without the "Input Monitoring" permission? **unknown**.
