---
tags: [protocol, p25]
status: draft
source: Synapse 4 background-manager (index-42d42381.js, otfm-3465588a.js, rzDevice25*.js)
---
# Protocol 2.5 — report format

Back to [[Protocol/Overview]]. Command list: [[Protocol/Commands 2.5]]. RE log: [[RE-Log/2026-11-09 Protocol 2.5]].

Tags: **observed** = read directly in Synapse 4 code; **inferred** = deduced from code + context (or OpenRazer); **unknown** = not determinable from the JS.

## Transport

| Item | Value | Tag |
|---|---|---|
| USB transfer | HID class **SET_REPORT** (bRequest 0x09) to send, **GET_REPORT** (bRequest 0x01) to read back, recipient = interface | observed (WebUSB fallback path in base class `rzDevice25`, `sendCommand` / `_getUSBTransferInResult`) |
| wValue | 0x0300 → report type Feature, report ID 0 | observed |
| wIndex | interface number from device config (`claimInterface`; 0 for Viper V3 HyperSpeed) | observed |
| Payload length | **90 bytes** (no report-ID byte in the WebUSB path) | observed |
| Length incl. report ID | **91 bytes** (`reportLength`) on the Electron/native path, report ID 0 prepended | observed |
| Native path | Electron IPC actions `hid.sendFeatureReport` / `hid.sendFeatureReportMutex` (mutex variant on macOS and engine ≥ 4.0.174), `hid.getFeatureReport`, tagged `protocol: "25"` | observed |
| Connect probe | on connect the native layer receives a pre-built *Get Firmware Version* (0x00/0x81) report as `queryData`; on Windows the leading byte is the interface number instead of the report ID | observed |

## Report layout (90 bytes, offsets without report ID)

| Offset | Size | Field | Request | Response | Tag |
|---|---|---|---|---|---|
| 0 | 1 | status | always 0x00 ("new command") | status code (below) | observed |
| 1 | 1 | transaction ID | rolling counter + routing bits (below) | echoed, must match | observed |
| 2–3 | 2 | remaining packets (u16 BE per OpenRazer) | always 0x0000 | not checked | observed (value) / inferred (meaning) |
| 4 | 1 | protocol type | always 0x00 | not checked | observed |
| 5 | 1 | data_size | from command table, per-call override | valid returned arg bytes | observed |
| 6 | 1 | command class | | echoed, must match | observed |
| 7 | 1 | command ID (bit 7 = get) | | echoed, must match | observed |
| 8–87 | 80 | arguments | first `data_size` bytes copied, rest 0 | data = bytes 8 .. 8+data_size | observed |
| 88 | 1 | CRC | XOR of bytes 2..87 | not verified by Synapse | observed |
| 89 | 1 | reserved | 0x00 | — | observed |

This **confirms** the OpenRazer layout. Each Synapse command is a 3-tuple `[default data_size, class, id]`; get = set ID | 0x80 almost everywhere (observed; one exception: class 0x18 DND uses get 0x85 / set 0x15).

### data_size override
Variable-length commands set byte 5 explicitly: DPI stages `7·n+3`, Chroma effect `6+3·colors`, Chroma frame `5+3·LEDs`, legacy matrix row `4+3·LEDs`. Bytes beyond `data_size` are never copied (observed).

## CRC
8-bit XOR of bytes 2..87 inclusive, stored at 88. Status (0) and transaction ID (1) excluded — observed. Synapse never checks the CRC of replies — observed.

## Status codes (response byte 0)

| Value | Name | Synapse handling | Tag |
|---|---|---|---|
| 0x00 | New Command | failure, stop polling, resend SET | observed |
| 0x01 | Command Busy | poll GET again (≤ maxRetryIn), then resend | observed |
| 0x02 | Command Successful | done | observed |
| 0x03 | Command Failure | failure, resend SET (≤ maxRetryOut) | observed |
| 0x04 | No Response / Timeout | failure, resend; ignored for *Set Device Mode* when configured | observed |
| 0x05 | Command Not Supported | failure, no resend | observed |
| 0x06 | Profile Not Supported | failure, no resend | observed |
| 0x07 | Target ID Not Supported | same | observed |
| 0x08 | Command Grab Control | same | observed |
| 0x09 | Not Supported in current Device Mode | same | observed |
| 0x0A | Command Broadcast | same | observed |
| 0x0B | Command Unknown | same | observed |

Mismatch of transaction ID, class or command ID in the reply → reply discarded, SET resent (observed).

## Transaction ID (byte 1)
Synapse does **not** use a fixed ID (OpenRazer: 0x1F/0x3F/0xFF). It uses a per-device-instance **rolling counter** 0x00…0x1E (wraps after 0x1E, never emits 0x1F) plus fixed **routing bits**:

| rzDevice class | Byte 1 | Tag |
|---|---|---|
| `rzDevice25` (base / wired) | counter | observed |
| `rzDevice25DualLinkMouse` (Viper V3 HyperSpeed) | counter; **0x80 \| counter** only for *Set Device Pairing Mode* (0x00/0x41) and *Set Device Scan* (0x00/0x46) | observed |
| `rzDevice25DualLinkKeyboard`, `rzDevice25WirelessKeyboard` | 0x80 \| counter | observed |
| `rzDevice25MikaWirelessKeyboard` | 0xF0 \| counter | observed |
| `rzDevice30` | counter | observed |

Interpretation (inferred): upper bits address a target behind a receiver: 0x00-range = primary device (mouse), 0x80-range = secondary device on a shared dongle (keyboard) or the dongle itself (pairing/scan). OpenRazer's constants (0x1F, 0x3F, 0x9F, 0xFF) are consistent with "counter field saturated + routing bits". There is **no separate device-index byte** (observed).

## Timing and retries

| Parameter | Base default | Viper V3 HS manifest | Meaning | Tag |
|---|---|---|---|---|
| sleepTimeBetweenOut | 5 ms | 25 ms | between SET retries | observed |
| sleepTimeBetweenOutIn | 5 ms | 25 ms | after SET, before first GET | observed |
| sleepTimeBetweenIn | 5 ms | 25 ms | between GET polls while busy | observed |
| maxRetryIn | 10 | 20 → **clamped to 10** | GET polls per SET | observed |
| maxRetryOut | 20 | 20 | SET attempts | observed |

Send algorithm (observed):
1. Build report (counter++); take a per-device lock (dual-link classes lock per container, so mouse + keyboard on one dongle are serialized).
2. SET_REPORT; if written length ≠ 91 → retry.
3. Fire-and-forget calls (lighting frames) stop here.
4. Sleep OutIn, GET_REPORT; while busy sleep In and poll again.
5. Success → return; failure flagged "resend" → back to 2; otherwise error.

## Batch lighting (native fast path)
A batched stream format exists for lighting (header + per-report entries + sleep entries) but is force-disabled in current Synapse — observed. Not needed for OpenSynapse.

## Hardware events (interrupt IN)
Byte 0 = record type (0x05 notification, 0x09 notification from dongle's secondary device, 0x02 media keys, 0x04 Razer keys …), byte 1 = event ID, payload from byte 2. Mouse-relevant: 0x01 profile change, 0x02 DPI change (X/Y/Z u16 BE), 0x09 wireless change, 0x31 battery change, 0x35 wireless change v2 (+PID), 0x56 dongle connection quality, 0x58 polling-rate change — observed. Details: [[Protocol/Hardware Events]].

## Open points
- Interface/endpoint carrying event reports on the Viper — unknown.
- Whether firmware validates the CRC — unknown.
- Additional timing inside native `HID.node` — unknown, see [[Protocol/Windows Transport]].
