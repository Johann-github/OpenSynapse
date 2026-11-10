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

## Open questions
- Data for "off" (function 0?) and DPI down.

## Next
- ✅ CLI `key` (hardware-tested) and `dpi-up` (dry run only).
- Capture "off" and DPI down.
