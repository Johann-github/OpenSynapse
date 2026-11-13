# ADR-0010: Linux hotplug = udev netlink socket, `libc` already in the tree

**Status:** accepted (2026-11-13). Stage 3 part 3 of [[Architecture/Linux and macOS port]].
Confidence: **inferred**. It is checked by CI only and has not been tested on Linux hardware.

**Context:** The agent rescans devices when `opensynapse_hid::hotplug::watch` reports that a
Razer HID interface arrived or was removed. Bursts are merged into one rescan after
`SETTLE` (150 ms, `opensynapse-agent/src/core.rs`). Windows uses `CM_Register_Notification`
([[Decisions/ADR-0005 Agent IPC]]). On Linux the stub returned `Unsupported`, so the agent
re-enumerated on every `ListDevices`. [[Decisions/ADR-0002 Lean by design]] rules out polling and
timers, and every new crate has to be justified.

**Options:**
| | What | Vendor filter | New crate | Code |
|---|---|---|---|---|
| a | netlink `NETLINK_KOBJECT_UEVENT` socket, read in one thread | yes: `DEVPATH` holds `BBBB:VVVV:PPPP.NNNN` on add *and* remove | none (`libc` 0.2 is already pulled in by hidapi) | ~150 lines FFI + ~90 lines pure parser |
| b | inotify on `/dev`, `hidraw*` create/delete | no: the name has no VID. Add could read sysfs, but on remove the node is already gone, so every hidraw removal (keyboards, headsets…) would rescan | none (`libc`) | about the same |
| c | `udev` crate (`MonitorBuilder`) | yes | `udev` + `libudev-sys` (+ `pkg-config` at build time). hidapi links libudev itself through its own build script, not through `libudev-sys` | least own code, two more crates |

Within (a), there are two multicast groups:
- **group 1 (kernel):** always present, but it fires **before** udev has run its rules. With
  `SETTLE` at 150 ms the rescan can run before `uaccess` has granted the user access to
  `/dev/hidrawN`, so the open fails and nothing retries it.
- **group 2 (udev):** udevd re-broadcasts each event **after** its rules have run, so the node has
  its final permissions. It needs a running udevd, which every desktop with systemd/eudev
  has. Without one, the `uaccess` rule from stage 3 would not work either.

**Decision:** (a) with group 2.
- `crates/opensynapse-hid/src/linux_hotplug.rs`: `socket(AF_NETLINK, SOCK_RAW|SOCK_CLOEXEC,
  NETLINK_KOBJECT_UEVENT)` bound with `nl_groups = 2`, plus an `eventfd`. One thread named
  `hotplug` sleeps in `poll([socket, eventfd], -1)`, with no timeout and no timer. `recv` is
  non-blocking after `poll` reports readiness.
- **Stop:** `Drop` writes 1 to the eventfd and joins the thread. That keeps the semantics of
  the Windows API: a running callback finishes first, and the watch must not be dropped from
  inside the callback. Both fds are `OwnedFd`, and the eventfd closes only after the join.
- **Pure parser:** `crates/opensynapse-hid/src/uevent.rs` is compiled on every platform and
  unit-tested on Windows.
  - Kernel format: `ACTION@DEVPATH\0KEY=VALUE\0…`.
  - udev format: a binary header that starts with `libudev\0`, then `magic` (u32 big-endian
    `0xfeedcafe`), `header_size`, `properties_off` and `properties_len` (u32 native-endian at
    bytes 12/16/20), then hash/bloom words. The parser reads only the
    `properties_off..+properties_len` block, so header bytes cannot corrupt the first key.
  - Source: the public systemd/libudev monitor code (`sd-device/device-monitor.c`, open
    source, not Razer). Confidence `inferred`.
  - It reports `ACTION=add` as `Arrived` and `ACTION=remove` as `Removed`, only for
    `SUBSYSTEM=hidraw` and only when the last `BBBB:VVVV:PPPP.NNNN` component of `DEVPATH`
    has VID = `vid`. `change`, `bind` and other actions are ignored.
  - `path` is `DEVNAME` as a node path, `/dev/hidrawN` (the kernel sends `hidrawN`, udev
    sends the full path). This is the same string hidapi's hidraw backend uses as the device
    path.
- **Dependency:** `libc = "0.2"` becomes a direct dependency, for Linux only. It is already
  locked through hidapi, so `Cargo.lock` gets no new package. Hand-written `extern "C"`
  declarations were rejected because the `sockaddr_nl`/`pollfd` layouts and the constants
  would be one more thing to get wrong without a Linux machine to test on.
- **Unsafe:** also allowed in `opensynapse-hid/src/linux_hotplug.rs`
  (`#![allow(unsafe_code)]`). Every block states its invariant, as in the Windows modules.

**Not done / accepted risks:**
- No `SCM_CREDENTIALS` sender check (libudev accepts udev messages only from uid 0). A
  spoofed message can only cause an extra rescan, which is harmless.
- On receive-buffer overrun (`ENOBUFS`) the watcher logs a warning and keeps going. Lost
  events are not replayed, and the next hotplug event or an agent restart fixes the device
  list. The default buffer is far larger than one device's burst.
- If the socket cannot be opened (e.g. a sandbox without netlink), `watch` returns an error
  and the agent falls back to rescanning on `ListDevices`, as before.
- macOS stays `Unsupported` (IOKit notifications, a later stage).

**Consequences:** No new crate. Idle cost on Linux is one sleeping thread with a 16 KiB
buffer. Hardware check (stage 7): plug the Viper's dongle in and out while the agent runs
with `RUST_LOG=debug`, and expect one `hotplug` debug line per hidraw node and one rescan.
