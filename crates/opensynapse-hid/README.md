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
- `conflicting_process()` warns when `RazerAppEngine.exe` runs (ToolHelp snapshot).
- `hotplug::watch`: arrival/removal of Razer interfaces without polling. Windows:
  `CM_Register_Notification` (`src/windows_hotplug.rs`, [[Decisions/ADR-0005 Agent IPC]]).
  Linux: udev uevents on a netlink socket, one thread in `poll` with an eventfd for stop
  (`src/linux_hotplug.rs`, pure parser `src/uevent.rs`,
  [[Decisions/ADR-0010 Linux hotplug]], confidence inferred). macOS: `Unsupported`.

Implements [[Protocol/Windows Transport]] and the timing part of [[Protocol/Report Format 2.5]].
Vault: [[Architecture/Crates]]
