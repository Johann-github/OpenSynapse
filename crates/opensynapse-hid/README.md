# opensynapse-hid

HID transport and device sessions. Windows: native SetupAPI + hid.dll (`src/windows.rs`,
[[Decisions/ADR-0003 Own Windows HID backend]]). Linux/macOS: hidapi.

- Picks the control collection: VID 0x1532, interface 0, usage page 0x0001 / usage 0x0002.
- Opens it with zero access on Windows and sends/gets 91-byte feature reports; on Windows
  the collection must also report a 91-byte feature report.
- `Session::execute` runs Synapse's send algorithm: SET → 25 ms → GET, poll while busy,
  resend on failure or mismatched reply, stop on permanent failure.
- `conflicting_process()` warns when `RazerAppEngine.exe` runs (ToolHelp snapshot).

Implements [[Protocol/Windows Transport]] and the timing part of [[Protocol/Report Format 2.5]].
Vault: [[Architecture/Crates]]
