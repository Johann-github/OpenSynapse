# opensynapse-desktop

Optional native GUI (egui via eframe, OpenGL renderer `glow`). Binary: `opensynapse-desktop`.
First slice: talks to the device in-process on a worker thread; a background agent with
IPC comes later ([[Architecture/Crates]]).

- Left: control collections from `opensynapse-hid` with names from the device registry.
  Only protocol 2.5 devices can be selected.
- Right, grouped: Device (firmware, serial, mode, battery, charging), DPI (current, stages
  with the active one in brackets), Polling, Power (sleep timer, raw value), Buttons
  (button 4/5 on profile 1, normal mode; read-only; reply byte 2 of 02/8C, meaning
  unknown, is not shown). "Refresh" reads everything again.
- Writes: DPI (volatile), polling rate, sleep timer. Same safety pattern as the CLI
  (`src/ops.rs`): check against the registry range → refuse while `RazerAppEngine.exe`
  runs unless the confirm box is ticked → read old → write → read back → show the result
  and a "Restore" button for the old value. Unit-tested against a fake mouse.
- Report bytes come only from `opensynapse-protocol` builders.

Lean by design ([[Decisions/ADR-0002 Lean by design]]): eframe with
`default-features = false` + `glow` + `default_fonts` (+ `x11`/`wayland` on Linux). No
wgpu, no persistence, no clipboard/links, no network, no telemetry. No continuous
repaint: the window redraws on input, and the HID thread blocks on its job channel and
requests one repaint per finished job.

Release exe (Windows, 2026-11-10): 4.4 MB.
