# ADR-0003: Own Windows HID backend instead of hidapi

**Status:** accepted (2026-11-09)

**Context:** `hidapi` 2.6.7 `HidApi::new()` always enumerates **every** HID collection
on the system, opening each one and reading its strings. Its discovery switch only
affects libusb. Even opening one known path needs a `HidApi`, so every CLI call paid
~200 ms on the dev PC (37 collections). `opensynapse list` took 245–285 ms against the
200 ms budget in [[Decisions/ADR-0002 Lean by design]]. On top of that, the
`RazerAppEngine.exe` check spawned `tasklist` (~300 ms per device command).

**Decision:** On Windows, `opensynapse-hid` talks to the OS directly through
`windows-sys` (already in the tree as a hidapi dependency):
- SetupAPI lists HID interface **paths** only. Paths without `vid_1532` are skipped
  **before** any device is opened.
- Matching paths are opened with zero access (see [[Protocol/Windows Transport]]).
  `HidD_GetAttributes` gives VID/PID and `HidP_GetCaps` gives usage page, usage and
  feature report length. The control collection additionally has to report a
  91-byte feature report.
- `HidD_SetFeature` / `HidD_GetFeature` move the reports.
- The process check uses a ToolHelp snapshot instead of spawning `tasklist`.
- `unsafe` is allowed only in `crates/opensynapse-hid/src/windows.rs`. Every block
  states its invariant. The workspace keeps `unsafe_code = "deny"`.
- hidapi stays the backend for Linux/macOS.

**Result (dev PC, release build):**
| | Before | After |
|---|---|---|
| `opensynapse list` | 245–285 ms | 75–84 ms |
| `opensynapse dpi get` | ~600 ms | 133–162 ms |
| Binary | 1.29 MB | 1.16 MB |

**Consequences:** About 250 lines of FFI code to maintain on Windows, offset by one
fewer runtime library there. Linux/macOS are not compile-checked on this machine yet.
