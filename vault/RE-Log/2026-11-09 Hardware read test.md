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

### Second run (battery, DPI, stages, polling, sleep)
All replies status 0x02 on the first GET, echo + valid CRC as above (observed).

| Command | Request args | Reply args | Decoded |
|---|---|---|---|
| 07/80 battery | `00` | `00 48` | raw 72 → 28 % |
| 07/84 charging | `00` | `00 00` | not charging |
| 04/85 DPI | `00` (storage 0) | `00 03 20 03 20 00 00` | 800 × 800, Z = 0 |
| 04/86 stages | `00` | `00 02 02 01 01 90 01 90 00 00 02 03 20 03 20 00 00` | 2 stages: id 1 = 400, id 2 = 800; active id 2 |
| 00/85 polling period | — | `01` | 1000 Hz |
| 07/83 sleep time | — | `01 2C` | 300 |

- (observed) The Viper V3 HyperSpeed on its dongle answers the **period** polling command
  00/85. Whether it also answers HyperPolling 00/C0 is untested.
- (observed) The DPI-stage reply layout matches [[Protocol/Commands 2.5]]: storage, active
  stage *ID*, count, then 7-byte stages (id, X, Y, Z).
- (observed) Storage byte 0 is echoed in DPI/stage replies.
- (inferred) The sleep time is in seconds: 300 = 5 min, a typical default.
- (observed) Only 2 stages are stored, not Synapse's 5 defaults, so the device holds the
  user's Synapse configuration.

### First write (DPI, volatile)
Synapse closed. `opensynapse -v dpi set 1600 --apply`:

| Step | Request args | Reply args |
|---|---|---|
| 04/85 read | `00` | `00 03 20 03 20 00 00` (800) |
| 04/05 set | `00 06 40 06 40 00 00` | same bytes echoed, status 0x02 |
| 04/85 read-back | `00` | `00 06 40 06 40 00 00` (1600) |

- (observed) Set DPI with storage 0 takes effect immediately (cursor speed changes) and
  reads back on the next 04/85 with storage 0.
- (observed) A set reply echoes the request arguments.
- (observed) Whole read → write → read-back took ~90 ms, no busy status.

## Open questions
- Does a storage-0 DPI set survive a power cycle (expected: no)?
- Device mode with Synapse closed (normal = 0?).
- Does 00/C0 (HyperPolling) also answer on this PID? (Only relevant with the 8 kHz dongle.)
- Storage 0 vs 1 on reads: does `dpi get` with storage 1 return a different value?
- Each CLI call takes ~600 ms; the `tasklist` check dominates (see [[Status]]).

## Next
- Restore 800 DPI; test storage 1 (`--persistent`) and a power cycle.
