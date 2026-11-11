# opensynapse-desktop

Optional native GUI (egui via eframe, OpenGL renderer `glow`). Binary: `opensynapse-desktop`.
Uses the background agent ([[Architecture/Agent]], [[Decisions/ADR-0005 Agent IPC]]) when
it runs, else talks to the device in-process on a worker thread ([[Architecture/Crates]]).

- Backend (`src/backend.rs`, `src/worker.rs`): at start `Client::connect()`; no agent
  (`NotFound`) means direct HID. The top bar shows "Backend: agent (pid N)" or "Backend:
  direct HID", and on direct HID a "Reconnect to agent" button (no automatic retries).
  With the agent, list/read/write go over IPC (`src/remote.rs` converts wire types to the
  GUI model); the agent keeps the same guards. A second thread holds a `Subscribe`
  connection: device arrival/removal lists devices again, battery and DPI events update
  the panel in place. When the subscription or a control call breaks, the GUI switches to
  direct HID and says so.
- Agent controls (`src/agent_ctl.rs` pure state/text, [[Decisions/ADR-0006 Agent autostart]]):
  on direct HID a green "Start agent" button next to "Reconnect to agent". The worker
  thread spawns the agent next to the GUI (`opensynapse_sys::spawn_agent_detached`, shared
  with `opensynapse agent start`), waits at most 3 s for its pipe (startup only), then
  switches to "Backend: agent". Exit code 3 shows "the agent refuses to run as
  administrator"; other failures show as the backend note. Below the bar a "Start agent
  at login" checkbox reads the per-user Run key at start and after each toggle (no
  polling) and calls `autostart::enable`/`disable`. An entry for another binary shows a
  warning and "Fix" (re-point it at this installation). Without the agent binary next to
  the GUI the checkbox and button are disabled with a tooltip; errors are status lines.

- Settings (`src/user_settings.rs`, [[Decisions/ADR-0008 User settings and wake hint]]): a "Settings" row under the autostart
  checkbox with "Show hint when the mouse is asleep" (default off). It reads
  `%APPDATA%\OpenSynapse\settings.toml` once at start and writes it on each toggle (same
  file as `opensynapse settings`). When on, a snapshot error or "Write not done" line of the
  no-response class gets "Mouse seems asleep — move it to wake it, then retry" below it. An
  invalid file disables the checkbox and shows why; it is never overwritten.

- Left: control collections from `opensynapse-hid` with names from the device registry.
  Only protocol 2.5 devices can be selected.
- Right, grouped: Device (firmware, serial, mode, battery, charging), DPI (current, stages
  with the active one in brackets), Polling, Power (sleep timer, raw value), Buttons.
  "Refresh" reads everything again.
- Buttons: all buttons of profile 1 from 02/8E, read once for normal and once for
  HyperShift mode, as a table (Left/Right/Wheel click, Side 4/5, Wheel up/down, DPI
  button, else "Button N"). Each slot has an editor: mouse button 1-5, double-click,
  key (`hid_usage` names + Ctrl/Shift/Alt/Win), DPI up; anything else shows as "Other
  (read-only)" (e.g. the DPI button's factory record, function 6 data `[6]`). Apply
  writes one slot (02/0C). Left/right click are locked until "allow editing left/right
  click" is ticked.
- DPI stages: edit values (registry min/max/step), add/remove (up to the reported count,
  at least 5, at most the protocol's 11), pick the active stage, Apply (04/06),
  volatile unless "Persistent" is ticked.
- Writes: DPI (volatile), DPI stages, polling rate, sleep timer, button slots. Same
  safety pattern as the CLI (`src/ops.rs`): check against the registry range → refuse
  while `RazerAppEngine.exe` runs unless the confirm box is ticked → read old → write →
  read back → show the result and a "Restore" button for the old value. Restore writes
  back exactly the bytes read (also for unknown button functions and locked buttons).
  Unit-tested against a fake mouse.
- Report bytes come only from `opensynapse-protocol` builders.

Lean by design ([[Decisions/ADR-0002 Lean by design]]): eframe with
`default-features = false` + `glow` + `default_fonts` (+ `x11`/`wayland` on Linux). No
wgpu, no persistence, no clipboard/links, no network, no telemetry. No continuous
repaint: the window redraws on input, and the HID thread blocks on its job channel and
requests one repaint per finished job.

Release exe (Windows, 2026-11-11): 4.6 MB (with IPC client and agent controls).
