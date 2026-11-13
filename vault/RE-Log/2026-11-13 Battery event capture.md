# 2026-11-13 Battery event capture

**Question:** Does the Viper V3 HyperSpeed (AA battery, no charging) send a battery-level
hardware event on its own, and does it match [[Protocol/Hardware Events]] event 49 (`0x31`)?
**Evidence (source / tool):** `opensynapse --pid 00B8 hid listen --all-readable --seconds 14400`
(read-only, installed CLI), started 15:52, Synapse closed, device mode Normal, mouse in normal
use. `opensynapse battery` before and after. Raw log kept outside the repo.

## Findings
- (observed) Battery before: `25% (raw 64)` at 15:52; after: `24% (raw 62)` at 18:47.
- (observed) One report in ~2 h 28 min of listening (none of the 8 collections sent anything else),
  at t+8857 s (≈ 18:20) on **MI_01 COL06**:
  `05 31 3E 00 00 01 00 00 00 00 00 00 00 00 00 00`
  - b0 `05` record/report 5, b1 `31` = event 49 battery change.
  - b2 `3E` = 62 = level 0–255; ⌊62/255·100⌋ = 24 %, same as the 07/80 read (`raw 62`).
  - b3 `00` charging = not charging, b4 `00` external power = no, b5 `01` device type = mouse.
  - Matches the event-49 layout from the Synapse parser exactly; the existing decoder printed
    `Battery { level: 62, charging: 0, external_power: false, device_type: 1 }`.
- (observed) No periodic heartbeat: the level fell by 2 raw steps (64 → 62) but only one event
  arrived in the window.
- (inferred) The device reports a level change, not every raw step; the step size or threshold is
  unknown (one sample). The 64 → 63 step may have happened before the listen started.
- The listen ended at ~18:20+ (stopped by the host for low memory, not by the device); no events
  were lost to the reader itself.

## Open questions
- Granularity: does the event fire per percent, per raw step, or only at some thresholds?
  Needs a longer capture with `battery` reads around each event.
- Low-battery event / warning threshold (07/xx low-power setting) still untested.

## Next
- Agent/GUI can update the battery display from event 49 instead of reading 07/80 (decoder exists).
