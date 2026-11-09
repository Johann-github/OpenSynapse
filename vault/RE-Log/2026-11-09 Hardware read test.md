# 2026-11-09 Hardware read test (Viper V3 HyperSpeed)

**Question:** Does the protocol 2.5 framing from [[Protocol/Report Format 2.5]] work on the real
device through plain hidapi (see [[Protocol/Windows Transport]])?
**Evidence (source / tool):** `opensynapse -v info` on the dev machine, PID 0x00B8 via HyperSpeed
dongle, MI_00 UP 0x0001/U 0x0002. RazerAppEngine.exe was running at the same time.

## Findings
- (observed) hidapi opens the MI_00 mouse collection and feature reports go through. No Razer
  driver API is involved.
- (observed) 00/81 firmware, 00/82 serial and 00/84 device mode all returned status 0x02
  (success) on the first GET, ~28 ms after SET (25 ms pacing + I/O). No busy polls.
- (observed) Replies echo transaction ID (rolling 0x00, 0x01, 0x02), class, command ID and
  data_size of the request.
- (observed) Reply CRC is valid: the device computes the XOR of bytes 2..87 on its replies.
- (observed) Firmware reply `[0x01, 0x00]` → shown as 1.00.
- (observed) Serial reply is 15 printable ASCII bytes, NUL-padded to 22 (value not recorded here).
- (observed) Device mode reply `[0x03, 0x00]` = driver mode, param 0.
- (inferred) Driver mode is set by the running Synapse; to confirm, read again with Synapse closed.

## Open questions
- Device mode with Synapse closed (normal = 0?).
- Remaining read commands: battery 07/80, charging 07/84, DPI 04/85, stages 04/86,
  polling 00/85 vs 00/C0, sleep 07/83.

## Next
- Run `opensynapse -v battery`, `dpi get`, `dpi-stages`, `polling get`, `sleep get`.
