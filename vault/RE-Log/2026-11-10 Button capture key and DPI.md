# 2026-11-10 Button capture key and DPI

**Question:** Which data does 02/0C carry for a keyboard key, DPI up and off?
**Evidence (source / tool):** USBPcap3 capture `~/re-work/button-remap-3.pcapng` (not in repo), Synapse 4 → Viper V3 HyperSpeed dongle, device address 9. Decoded with tshark (`usb.data_len == 98`). Builds on [[RE-Log/2026-11-09 Button remap capture]].

## Findings
All frames: class 02, command 0C, data_size 10, args `[profile, button, layer, function, dataLen, data…]`.

| t (s) | Synapse action | args | function | data |
|---|---|---|---|---|
| 68.7, 79.9 | button 4 → key A | `01 04 00 02 02 00 04` | 2 | `[00, 04]` |
| 118.6 | button 4 → DPI up | `01 04 00 06 01 01` | 6 | `[01]` |
| 124.6 | button 4 → default | `01 04 00 01 01 04` | 1 | `[04]` |

- (observed) Keyboard = function 2 with two data bytes. Synapse sent the frame twice.
- (inferred) Key data = `[modifiers, HID usage]`; 0x04 is the HID usage ID for A, modifiers 0 = none.
- (observed) DPI up = function 6, data `[01]`.
- (inferred, not seen) DPI down = function 6, data `[02]`.
- (unknown) "Off": no frame in this capture.
- (observed) Before the writes, Synapse polls 02/8E with arg 01 about 20 times (meaning unknown), then reads 00/84, 07/84, 07/80, 04/93, 04/87, 50/BF, 03/C5, 07/83.
- (observed) After the key-A write the user reported that the Razer mouse no longer moved the cursor during the capture. Afterwards the control interface still answered (`info`: device mode Driver, `buttons get 4` = default). Cause: USBPcap3 hang of the dongle (replug fixed it). Not the key assignment: the same key-A write via `opensynapse buttons set 4 key a --apply` with Synapse and Wireshark closed typed `a` and the cursor kept moving (observed 2026-11-10). Restored to mouse button 4.

## Hardware test DPI up (2026-11-10, Synapse closed)
- (observed) `buttons set 4 dpi-up --apply`, verified. Stages 400/800/1600/3200/6400. Start: DPI 1600.
- (observed) First press: DPI 6400, active stage 5. That is two steps instead of one (user: pressed once).
- (observed) Second press: DPI stays 6400. The highest stage is a limit; no wrap-around.
- (inferred) Either press and release each trigger one step, or the first press was registered twice. Not resolved.
- (observed) `dpi set 800` (storage 0 and 1) changes the DPI but not the active-stage index (still stage 5).
- Restored: button 4 = mouse button 4, DPI 800 (volatile + persistent).

## Retest DPI up (2026-11-10, `dpi-stages set --active`)
- (observed) `dpi-stages set --active 1` (04/06, storage 0) also applies the stage DPI: 04/85 went 800 → 400.
- (observed) One press = one step: stage 1/400 → stage 2/800 → stage 3/1600.
- (inferred) DPI up steps from the stored active-stage index, not from the current DPI. The "double step"
  before was stage 4 → 5 while 04/85 showed 1600 (DPI set separately by 04/05).
- (observed) The persistent stage list (storage 1) keeps its own active index (it was 3 before the restore).
- Restored: button 4 = mouse button 4, active stage 2 (volatile + persistent), DPI 800.

## Layer byte puzzle (2026-11-10, `profile export`)
- (observed) GET 02/8C `01 01 00` → reply `01 01 01 01 01 01`; GET `01 01 01` → same reply.
- (observed) GET `01 04 00` and `01 04 01` both → `01 04 00 01 01 04`.
- (observed) Buttons 1, 2, 3, 5 (never written) reply third byte 1; button 4 (written with 0) replies 0.
- ~~(inferred) The device ignores the third byte on GET~~ wrong, see "Resolved" below.
- RE check (observed in code, Synapse 4 rzDevice25): Synapse also calls byte 2 `mode` (0 normal, 1 HyperShift),
  sends it on GET and expects it echoed. So the naming is right; the Viper firmware just does not honour it on GET.
  The captures contain no mode-1 write and no 02/8C/8E reads by Synapse. Per-device UI: `OBMSlots: 1`,
  mappings carry `isHyperShift`; turning that into bytes happens in the native mapping engine (not visible).
- (observed, hardware) 02/8E `[1, 0, 0]` and `[1, 1, 0]` give identical data. Records of 8 bytes:
  `01 01 01 01 …`, `02 01 01 02 …`, `03 01 01 03 …`, `04 01 01 04 …`, `05 01 01 05 …`, `60 06 01 06 …`, `09 01 01 09 …`, `0a 01 01 0a …`.
- (inferred) Record = `[buttonId, functionId, dataLen, data (5 bytes)]`. Button 0x60 (96, not in Synapse's
  button enum) = function 6 (DPI) data `[06]`, probably the DPI button on top (cycle stages; data 6 unknown).
  Wheel up/down (9/10) = mouse buttons 9/10. (At that point both modes held the same defaults.)
- **Resolved (observed, hardware):** `buttons set 5 mouse-button 4 --hypershift --apply` (mode-1 SET) →
  GET `01 05 01` → `01 05 01 01 01 04` (button 4), GET `01 05 00` → `01 05 01 01 01 05` (unchanged).
  02/8E mode 1 showed `05 01 01 04`, mode 0 still `05 01 01 05`. Restored to mouse button 5.
- So the Viper **has separate HyperShift slots and GET honours the mode**. The earlier "ignores the mode"
  inference was wrong: both modes just have the same factory defaults. Reply byte 2 is not a mode echo
  (button 5 replies 1 in both modes; meaning unknown).
- Code fixed: no experimental HyperShift gate; `profile export` writes HyperShift entries that differ from normal.
- (observed) `profile apply` of an exported (identical) profile with `--apply`: 8 writes (stages, polling, sleep,
  buttons 1–5), each verified by read-back; restore snapshot written first. Mouse state unchanged.
- Effect: `profile export` puts buttons 1/2/3/5 under `[hypershift.buttons]`. Do not use `profile apply`
  for buttons until this is resolved.

## Open questions
- Data for "off" (function 0?) and DPI down.

## Next
- ✅ CLI `key` and `dpi-up` hardware-tested; dpi-up = one stage per press.
- Capture "off" and DPI down.
