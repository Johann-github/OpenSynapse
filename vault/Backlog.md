---
tags: [moc, backlog]
updated: 2026-11-12
---
# Backlog (later)

Parked items, ranked. Pick from here when [[Status]] "Resume here" is empty.

## Reverse engineering
- ✅ Battery-level event observed 2026-11-13 (`05 31 …`, [[RE-Log/2026-11-13 Battery event capture]]).
  Still open: its granularity (per % or thresholds), low-battery warning.
- ~~**Battery-level events (07/xx, event report 5).**~~ The Viper V3 HyperSpeed runs on one AA battery
  and cannot charge ([[Devices/Viper V3 HyperSpeed]]), so charging events are out. Level events:
  leave `opensynapse hid listen` running over a longer session, or test with a rechargeable device.
  → [[RE-Log/2026-11-10 Hardware events capture]]
- **81 products without a manifest** → protocol unknown in `data/devices/`. Only matters for
  devices other than the Viper. → [[Devices/Device Registry]]
- Polling command 00/40 (HyperPolling) untested; the Viper uses 00/05.
- Viper: does the USB cable carry data (own PID)? Unit/range of the low-power threshold.

## Platforms
- **Linux / macOS** (goal: second). Missing there: HID transport (hidapi path exists, untested on
  hardware), autostart, foreground watch for per-app profiles (returns `Unsupported`), install location.

## Ideas, declined for now
- Windows toast when an automatic write (app-profile switch) fails because the mouse sleeps.
  User 2026-11-11: not wanted; the opt-in hint in CLI/GUI is enough ([[Decisions/ADR-0008 User settings and wake hint]]).
