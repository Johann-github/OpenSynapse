# 2026-11-10 Onboard button test

**Question:** Does an onboard button assignment written by OpenSynapse work with Synapse closed?
**Evidence (source / tool):** hardware test on Viper V3 HyperSpeed (PID 0x00B8, dongle), `opensynapse buttons get|set` (commit 5ed0db9). Builds on [[RE-Log/2026-11-09 Button remap capture]].

## Findings
- (observed) `buttons get 4` (02/8C, profile 1, button 4, layer normal) returns function 1, data `[04]` = mouse button 4, the default.
- (observed) The dry-run frame for button 4 = double-click is byte-exact with the Synapse capture: args `01 04 00 0b 01 01`, data_size 10, CRC 0x0a.
- (observed) With `RazerAppEngine.exe` closed, the device mode reads `Normal (param 0)` (not driver mode).
- (observed) After `buttons set 4 double-click --apply` (read-back verified), a single press of button 4 opened a folder in Explorer. The firmware performs the double-click by itself; no host software is needed.
- (observed) Restore with `buttons set 4 mouse-button 4 --apply`, read-back verified.
- (inferred) The write goes to profile 1 in onboard memory. Persistence across a power cycle is untested.

## Open questions
- Does the assignment survive a power cycle (like DPI storage 1)?
- Data layouts for keyboard key, DPI up/down and off (function 0) → more captures.

## Next
- Captures in Synapse: button → keyboard key, DPI up, off. See [[Status]].
