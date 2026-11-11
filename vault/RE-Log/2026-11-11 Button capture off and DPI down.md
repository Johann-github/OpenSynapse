# 2026-11-11 Button capture off and DPI down

**Question:** Which 02/0C data does Synapse send for "Disable" and "DPI down"?
**Evidence (source / tool):** USBPcap3 capture `~/re-work/button-remap-4.pcapng` (not in repo), Synapse 4 → Viper V3 HyperSpeed dongle, device address 9. Decoded with tshark (`usb.data_len == 98`). Follows [[RE-Log/2026-11-10 Button capture key and DPI]].

## Findings
All frames: class 02, command 0C, data_size 10, args `[profile, button, layer, function, dataLen, data…]`.

| t (s) | Synapse action | args | function | data |
|---|---|---|---|---|
| 18.6 | button 4 → Disable | `01 04 00 00 00` | 0 | none |
| 34.5 | button 4 → DPI down | `01 04 00 06 01 02` | 6 | `[02]` |
| 40.8 | button 4 → default | `01 04 00 01 01 04` | 1 | `[04]` |

- (observed) Off = function 0, dataLen 0. Confirms the "0 off" guess in the protocol docs.
- (observed) DPI down = function 6, data `[02]` (was inferred before).
- (observed) Each frame was sent once in this capture (key A was sent twice before).

## Hardware test (2026-11-11, Synapse closed, OpenSynapse CLI)
- (observed) `buttons set 4 dpi-down --apply`, read back OK. Stages 400/800, active 2, DPI 800.
  One press → active stage 1, DPI 400.
- (observed) `buttons set 4 off --apply`, read back `off (function 0, data [])`. In a browser,
  button 4 (normally "back") did nothing (user-checked).
- Restored: button 4 = mouse button 4; active stage 2 and DPI 800 (volatile + persistent).

## Code
`opensynapse-protocol::buttons::{off, dpi_down}`, `Known::{Off, DpiDown}`; profile actions
`off` / `dpi-down`; CLI `buttons set <n> off|dpi-down`; GUI editor choices "DPI down", "Off (disabled)".

## Power-cycle test (2026-11-11, agent stopped)
- (observed) `buttons set 4 off --apply` (02/0C, profile 1, normal mode), then the user switched the
  mouse off for ~5 s and on again. Read back: button 4 still `off (function 0, data [])`;
  HyperShift slot unchanged (mouse button 4). So 02/0C writes are stored on the device (onboard
  memory), not volatile.
- (observed) Volatile DPI stages also came back as 400/800, active 2, DPI 800 (same as the
  stored list at that time, so this does not tell volatile from persistent).
- Restored: button 4 = mouse button 4.
