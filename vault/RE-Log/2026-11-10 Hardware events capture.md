# 2026-11-10 Hardware events capture

**Question:** Which HID collection carries the input report 5 events on the Viper V3 HyperSpeed?
**Evidence (source / tool):** `opensynapse hid collections --pid 00B8`, `opensynapse hid listen --all-readable` (read-only), `opensynapse-agent --events` + `opensynapse agent watch`. Synapse closed, device mode Normal. → [[Protocol/Hardware Events]]

## Findings
- (observed) Readable collections: MI_00 COL03/COL01 (0x000C/0x0001), MI_00 COL03/COL02 (0x0001/0x0080),
  MI_01 COL03 (0x000C/0x0001), MI_01 COL04 (0x0001/0x0080), MI_01 COL05–COL08 (0x0001/0x0000, input 16 bytes).
  The mouse (MI_00, 0x0001/0x0002) and keyboard (0x0001/0x0006) collections are exclusive to the OS (not readable).
- (observed) Three presses of the DPI button on the underside → three reports on **MI_01 COL06** only:
  `05 02 06 40 06 40 …` (DPI 1600), `05 02 0C 80 0C 80 …` (3200), `05 02 19 00 19 00 …` (6400).
  Layout: report id 5, event 0x02 = DPI changed, X and Y as u16 big-endian. Matches `protocol::event::decode`.
- (observed) Events arrive in device mode **Normal**; Synapse/driver mode is not needed.
- (observed) The DPI button (button 96, function 6 data `[06]`) cycles through the DPI stages onboard.
- (observed) The agent with events on: `agent watch` printed `DPI: 800`, `DPI: 1600` live for two presses.
  Agent with 4 reader threads: 9 threads, 8.8 MB working set, CPU 0.08 s after 39 s.
- (observed) A 45 s and a 90 s listen without presses stayed silent: no periodic reports (no battery
  heartbeat in that window).

## Open questions
- Battery / charging events: which event id, and when are they sent? ~~Plug in the charging cable during a listen.~~
  Not testable on the Viper V3 HyperSpeed: it runs on one AA battery and has no charging (user, 2026-11-11).
  Battery-level events may still come when the level changes; needs a long `hid listen` or another device.
  ✅ Observed 2026-11-13: [[RE-Log/2026-11-13 Battery event capture]].
- COL05/07/08 stay silent; maybe used by other event types or a second device.

## Next
- Events are on by default in the agent (`--no-events` to turn them off).
- Narrow the candidate collections to COL06 once battery events are checked.
