# 2026-11-11 Per-app profile hardware test

**Question:** Does the agent switch the Viper V3 HyperSpeed (PID 00B8) to an app profile when an
app comes to the front, and restore it afterwards? ([[Decisions/ADR-0007 Per-app profiles]])
**Evidence (source / tool):** release agent `-v` with a scratch rules file
(`notepad.exe` → profile with only `[dpi] stages = [500, 1000, 2000, 4000, 8000]`, `active_stage = 2`),
`opensynapse agent snapshot` before/while/after, agent log.

## Findings
- (observed) Foreground hook reports the packaged Notepad as
  `C:\Program Files\WindowsApps\Microsoft.WindowsNotepad_…\Notepad\Notepad.exe`; the file-name rule
  `notepad.exe` matches it.
- (observed) Switch: foreground event → profile written and verified in ~112 ms. Stages
  `[400, 800, 1600, 3200, 6400]` (active 3) → `[500, 1000, 2000, 4000, 8000]` (active 2), DPI read back 1000.
- (observed) Back to VS Code (no rule, no default): previous values restored and verified, DPI 1600.
- (observed) Only DPI stages were written (volatile storage); other settings absent from the profile were untouched.
- (observed) Windows 11 Notepad: `Start-Process notepad` returns a launcher PID that exits at once;
  the real window belongs to another process that reopens the previous session's tabs.

## Open questions
- Device plugged in while an app profile is active gets it only at the next switch.
- Polling/sleep/button switches write persistently (no volatile variant known) — not exercised here.

## Next
- GUI view for app profiles / the `AppProfile` event.
