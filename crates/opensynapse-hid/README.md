# opensynapse-hid

hidapi transport and device sessions.

- Picks the control collection: VID 0x1532, interface 0, usage page 0x0001 / usage 0x0002.
- Opens it via hidapi (zero-access fallback on Windows), sends/gets 91-byte feature reports.
- `Session::execute` runs Synapse's send algorithm: SET → 25 ms → GET, poll while busy,
  resend on failure or mismatched reply, stop on permanent failure.
- `conflicting_process()` warns when `RazerAppEngine.exe` runs.

Implements [[Protocol/Windows Transport]] and the timing part of [[Protocol/Report Format 2.5]].
Vault: [[Architecture/Crates]]
