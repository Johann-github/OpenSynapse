# opensynapse-hid

HID transport and device sessions. Windows: native SetupAPI + hid.dll (`src/windows.rs`,
[[Decisions/ADR-0003 Own Windows HID backend]]). Linux/macOS: hidapi.

- Picks the control collection: VID 0x1532, interface 0, usage page 0x0001 / usage 0x0002.
- Opens it with zero access on Windows and sends/gets 91-byte feature reports; on Windows
  the collection must also report a 91-byte feature report.
- `Session::execute` runs Synapse's send algorithm: SET → 25 ms → GET, poll while busy,
  resend on failure or mismatched reply, stop on permanent failure.
- Errors: `HidError::NoResponse` when the retries ran out and the last reply was status
  Timeout (0x04): a sleeping wireless mouse (observed) or one out of range (inferred).
  Its text starts with `NO_RESPONSE`; `is_no_response_text` recognises it after the error
  became a string. [[Decisions/ADR-0008 User settings and wake hint]]
- `input::spawn` reads input reports (hardware events, [[Protocol/Hardware Events]]) on one
  thread per collection; the callback gets byte 0 = report ID (0 if unnumbered) on every
  platform. Windows: overlapped `ReadFile`, no timeouts (`src/windows_input.rs`).
  Linux: the hidraw node (`/dev/hidrawN`, hidapi's path) opened read-only, one thread
  sleeping in `poll` on it plus an eventfd that `stop`/Drop write, then join; no timeouts,
  no idle wakeups (`src/linux_input.rs`, same pattern as hotplug). Unplug (POLLHUP/POLLERR
  or a read error such as ENODEV/EIO) ends the reader with an error. Report-ID handling
  comes from the report descriptor in sysfs (`/sys/class/hidraw/hidrawN/device/report_descriptor`).
  macOS: hidapi `read_timeout` (`src/hidapi_input.rs`); hidapi cannot unblock a read from
  another thread, so the thread wakes once per second without a report to check its stop
  flag (no device I/O for that), and `stop` takes up to 1 s. Open item: the one platform
  with an idle wakeup. Read buffer 1 KiB on both (no input length is known, so `input_len`
  stays `None` there). On Linux a hidraw node is a whole interface: the callback
  also sees the other collections' reports. macOS opens non-exclusively. Confidence
  inferred, untested on hardware ([[Architecture/Linux and macOS port]], stage 3).
- Enumeration keeps collections that share a path (hidapi lists each top-level collection
  of a hidraw node under one path).
- Protocol 4.0 (inferred, untested): the framing comes from the registry protocol of the
  PID (`framing_for_pid`). `Transport::open` builds `Session::with_framing`; the control
  collection is the configured interface with a 64-byte feature report (report ID 2).
  Same send algorithm; buffers are 64 bytes with report ID 2 in byte 0.
  [[Decisions/ADR-0015 Protocol 4.0 and 3.0 support]]
- `conflicting_process()` warns when `RazerAppEngine.exe` runs (ToolHelp snapshot).
- `hotplug::watch`: arrival/removal of Razer interfaces without polling. Windows:
  `CM_Register_Notification` (`src/windows_hotplug.rs`, [[Decisions/ADR-0005 Agent IPC]]).
  Linux: udev uevents on a netlink socket, one thread in `poll` with an eventfd for stop
  (`src/linux_hotplug.rs`, pure parser `src/uevent.rs`,
  [[Decisions/ADR-0010 Linux hotplug]], confidence inferred). macOS: `Unsupported`.

Implements [[Protocol/Windows Transport]] and the timing part of [[Protocol/Report Format 2.5]].
Vault: [[Architecture/Crates]]
