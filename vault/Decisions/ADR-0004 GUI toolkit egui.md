# ADR-0004: GUI toolkit = egui (eframe)

**Status:** accepted (2026-11-10, user decision)

**Context:** Phase 4 needs a GUI. [[Decisions/ADR-0002 Lean by design]] rules out
Electron/WebView and asks for a native, optional GUI with idle CPU ≈ 0 %.
Candidates: egui/eframe, Slint, iced, plain Win32.

**Decision:** `crates/opensynapse-desktop` uses egui via eframe with
`default-features = false`, the smallest renderer that works (glow preferred), and no web,
persistence or telemetry features.

**Why:**
- Pure Rust, cross-platform (Windows first, Linux/macOS later with the same code).
- Immediate mode with reactive repaint: it only redraws on input or `request_repaint()`,
  so idle CPU ≈ 0 % when no continuous repaint is requested.
- Small binary (estimate 4–8 MB, measure in the first slice) and the quickest to build.
- Rejected: Slint (licence terms), iced (bigger, more to learn), Win32 (Windows only).

**Consequences:**
- The UI looks like a tool rather than a native Windows app. That is acceptable.
- Device I/O runs off the UI thread; results trigger a repaint.
- Every write uses the CLI safety pattern: read old → write → read back → verify, with restore.
- Later the GUI talks to `opensynapse-agent` over IPC instead of opening HID itself.
