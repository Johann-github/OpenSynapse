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
  Linux/macOS: hidapi `read_timeout` (`src/hidapi_input.rs`); hidapi cannot unblock a read
  from another thread, so the thread wakes once per second without a report to check its
  stop flag (no device I/O for that), and `stop` takes up to 1 s. Report-ID handling comes
  from the report descriptor; read buffer 1 KiB (hidapi gives no input length, so
  `input_len` stays `None` there). On Linux a hidraw node is a whole interface: the callback
  also sees the other collections' reports. macOS opens non-exclusively. Untested on
  hardware ([[Architecture/Linux and macOS port]], stage 3).
- Enumeration keeps collections that share a path (hidapi lists each top-level collection
  of a hidraw node under one path).
- `conflicting_process()` warns when `RazerAppEngine.exe` runs (ToolHelp snapshot).

Implements [[Protocol/Windows Transport]] and the timing part of [[Protocol/Report Format 2.5]].
Vault: [[Architecture/Crates]]
