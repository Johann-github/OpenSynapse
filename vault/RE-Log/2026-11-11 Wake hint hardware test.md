# 2026-11-11 Wake hint hardware test

**Question:** Does a sleeping Viper produce `HidError::NoResponse`, and does the opt-in hint show only when enabled?
**Evidence (source / tool):** release build, Viper V3 HyperSpeed via dongle, sleep timer 300 s, Synapse closed, agent running. Implements [[Decisions/ADR-0008 User settings and wake hint]].

## Findings
- (observed) Mouse untouched for 6 min (19:39:53 → 19:45:59). `opensynapse dpi get` →
  `Error: no response from the device (status Timeout after 20 attempts)`, exit 1.
  So the sleeping mouse answers with status 0x04 Timeout on the last attempt; this confirms the
  ADR-0008 classification (was based on the slice-1 observation).
- (observed) With `wake-hint = on`: CLI prints `Hint: Mouse seems asleep — move it to wake it, then retry`
  after the error, both direct (`dpi get`) and via the agent (`agent snapshot`:
  `agent: device not responding: … no response from the device …`).
- (observed) With `wake-hint = off` (default): the same two commands print only the raw error.
- (unknown) Whether every one of the 20 attempts returned 0x04 (only the last reason is logged).
- Setting left at `off` (default) after the test.
