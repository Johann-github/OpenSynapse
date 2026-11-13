# Protocol 3.0 and 4.0

Synapse 4 (RazerAppEngine 4.0.896) has two newer command protocols besides
[[Protocol/Report Format 2.5]]. This note only covers how they **differ** from 2.5.
Evidence pointers: [[RE-Log/2026-11-09 Protocol 3.0 4.0 and events]].
Events (device→host) are in [[Protocol/Hardware Events]].

Each claim has a tag: **observed** (read directly in shipped code), **inferred**
(follows from the code but is not stated there), **unknown**.

## TL;DR

| | 2.5 | 3.0 | 4.0 |
|---|---|---|---|
| Transport | USB HID feature report | **Bluetooth LE GATT** (not USB HID) | USB HID feature report (`rzDevice40`); also a BLE variant (`rzDevice40Ble`) |
| Report ID | 0 | n/a (GATT characteristic) | **2** |
| Frame length | 90 B (+1 report ID = 91) | 8 B header + variable payload (24-bit length) | 63 B (+1 report ID = **64**) |
| Header | status, txn, size, class, id | txn, len24 LE, class, id, profile, region/status | status, txn(+target), size16 BE, class, id |
| Checksum | XOR | **none** | XOR (same algorithm as 2.5, different position) |
| Command addressing | `[size, class, id]` tuple | same tuple; header also carries profile + region | same tuple **and the same command tables as 2.5** |
| Multi-packet | no | yes (BLE fragmentation) | no evidence; max 53 argument bytes per frame |

(observed for every row except "no evidence" = unknown)

## How Synapse picks the protocol

- **observed**: No runtime negotiation. Each product has a downloaded device manifest
  (`lighting-engine/manifest/DeviceManifest_<pid>_<edition>.json`). Its
  `rzDeviceConfig.rzDeviceName` names the JS device class. The lighting-engine has a
  loader table that maps that name to a code chunk.
- **observed**: Class names in that table, grouped by family:
  - 2.5: `rzDevice25`, `rzDevice25DualLinkMouse`, `rzDevice25LedMatrix*`, `rzDevice25Oled`, …
  - 3.0: `rzDevice30`, `rzDevice30V2`, `rzDevice30Layla`, `rzDevice30Oled`
  - 4.0: `rzDevice40`, `rzDevice40Ble`, `rzDevice40Headset`, `rzDevice40HeadsetAiroha`, `rzDevice40BillieT3H`
- **observed**: The same manifest has a numeric `hardwareEventParserType`, which picks the
  event parser on its own (see [[Protocol/Hardware Events]]).
- **observed**: Dev device Viper V3 HyperSpeed (PID `0x00B8` = 184) has
  `rzDeviceName = rzDevice25DualLinkMouse`, `claimInterface = 0`, `hardwareEventParserType = 1`
  (PROTOCOL25). **It speaks 2.5, not 3.0/4.0.**
- **observed**: A second table, the "on-the-fly macro" manifest in the `otfm` chunk
  (a Map keyed by PID), also maps PID → `{name: "razerProtocol25|razerProtocol30", className}`.
  Every `razerProtocol30` entry there is a keyboard (BLE-capable keyboards).
- **observed**: An internal design note in the Electron shell (`noble/strategy/spec-protocol30v2.md`)
  says the factory chooses between `rzDevice30` and `rzDevice30V2` "based on the firmware
  protocol version".
- **unknown**: Where that firmware version comes from. The 2.5 command table has
  "Get Protocol Version" (class `0x00`, id `0x83`), but nothing shows it being used
  to pick a protocol. In practice the manifest value decides.
- **OpenSynapse implication (inferred)**: ship a PID → {protocol class, event parser,
  claimInterface, reportId} table taken from the manifests. Do not probe.

## OpenSynapse status (2026-11-13)
- **4.0 (USB HID):** frame encode/decode in `opensynapse-protocol` (`report40`, `framing`),
  session/transport in `opensynapse-hid` (picks the framing from the registry protocol),
  `Protocol::is_supported()` in the registry is the one support gate. **Inferred, never run
  on hardware.** No registry device is 4.0 yet (see "Which devices speak 4.0").
- **3.0 (BLE GATT):** not supported. It is a vendor GATT service, not a HID collection, so
  the HID path cannot reach it. Plan in [[Decisions/ADR-0015 Protocol 4.0 and 3.0 support]].

## Protocol 3.0 (BLE GATT)

### Transport
- **observed**: `rzDevice30` sends nothing over USB. All I/O goes through the `rzBLE`
  helper to the Electron `noble` module (strategy `rzDevice30` / `rzDevice30V2` / `rzDevice30Layla`).
- **observed**: Default GATT UUIDs (Electron `noble/constants.js`):

  | Role | UUID |
  |---|---|
  | Service | `52401523-f97c-7f90-0e7f-6c6f4e36db1c` |
  | Write characteristic | `52401524-f97c-7f90-0e7f-6c6f4e36db1c` |
  | Read / Notify characteristic | `52401525-f97c-7f90-0e7f-6c6f4e36db1c` |

  A device class can override these UUIDs (`setUUID`).
- **observed**: Razer BLE vendor IDs that are accepted: `0x1532` and `0x068E`.
- **inferred**: The service UUID is vendor-specific (not the Bluetooth SIG HID service
  `0x1812`). Windows maps only the HID-over-GATT service to HID collections, so the 3.0
  command channel does **not** appear as a HID device and `HidD_SetFeature` cannot reach it.
  Synapse itself goes through `noble` (a GATT client), not `node-rz-hid`, which agrees.
  The keyboard/mouse input of such a device over Bluetooth is ordinary HOGP and is
  unrelated to configuration.
- **observed**: Command timeouts are 500 ms (legacy 3.0) and 1500 ms (V2).

### Request frame

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 1 | transaction id | Counter 0…30, then wraps (observed). The response echoes it (observed). |
| 1 | 3 | payload length | 24-bit **little-endian** (observed) |
| 4 | 1 | command class | `cmd[1]` (observed) |
| 5 | 1 | command id | `cmd[2]` (observed) |
| 6 | 1 | arg A | Usually **profile id** (observed in profile polling-rate and chroma calls) |
| 7 | 1 | arg B | Usually **region / target id** in requests (observed in chroma calls) |
| 8… | n | payload | Length = header length field |

- **observed**: There is no checksum and no padding. The frame is exactly `8 + payload` bytes.
- **observed**: Commands are written as the same 3-byte tuple used in 2.5:
  `[default payload size, class, id]`. A size of `255` marks a variable-length payload.
- **inferred**: The "get" variant of a command has bit 7 set on the id (e.g. id `0x02`
  set / `0x82` get for device mode). Most tuples in the module come in such pairs.

### Response frame
- **observed**: Same 8-byte header. Byte 0 = echoed transaction id, bytes 1–3 = payload
  length (LE), **byte 7 = status**, payload follows.
- **observed**: Status values (shared error-name table, index = status):

  | Value | Meaning |
  |---|---|
  | 0 | New command |
  | 1 | Busy (host retries) |
  | 2 | Success |
  | 3 | Failure (host retries once) |
  | 4 | No-response timeout |
  | 5 | Command / class not supported |
  | 6 | Profile not supported |
  | 7 | Target id not supported |
  | 8 | Grab control |
  | 9 | Not supported in current device mode |
  | 10 | Broadcast |
  | 11 | Unknown |

- **observed**: The device class first assumes a 20-byte response header. If the length
  field does not match, it falls back to the 8-byte header.
- **unknown**: Why the 20-byte header exists. It could be an older firmware variant.

### Multi-packet (fragmentation)
- **observed, legacy (`rzDevice30`)**: The host writes the 8-byte header first, then the
  payload in separate **20-byte** writes. The device answers with a notify that holds only
  the header, then sends the payload in follow-up notifies until the length is reached.
- **observed, V2 (`rzDevice30V2`)**: Each GATT write is `header + chunk`, up to
  `maxPacketSize` bytes (default **256**, so a 248-byte chunk). Larger payloads are split, and
  every fragment **repeats the same header**. The firmware reassembles them using the
  length field and sends one response, with header and full payload in a single notify.
  This needs an ATT MTU of at least 259.
- **observed, Layla variant (`rzDevice30Layla`)**: Like V2 on the receive side, with a
  big-endian length fallback.
- **unknown**: The exact Layla rules (only seen through the V2 spec's references).

### Worked example (byte values from the vendor spec in the Electron shell)

| Purpose | Direction | Bytes |
|---|---|---|
| Set device mode = 3 | OUT | txn `00`, len `02 00 00`, class `01`, id `02`, A `00`, B `00`, payload `03 00` |
| | IN | txn `00`, len `00 00 00`, `00 00 00`, status `02` |
| Get firmware version | OUT | txn `00`, len `00 00 00`, class `00`, id `81`, `00 00` |
| | IN | txn `00`, len `04 00 00`, `00 00 00`, status `02`, payload `major minor x x` |

### 3.0 command classes (observed: tuples in `rzDevice30`; names inferred from nearby description strings)

| Class | Area |
|---|---|
| `0x00` | Firmware version (`0x81`) |
| `0x01` | Device: mode (`0x02`/`0x82`), serial (`0x83`), polling rate (`0x04`/`0x84`), edition info (`0x86`), ambidextrous (`0x0A`/`0x8A`), wireless connection status, external power, dock charging |
| `0x03` | Profiles: list, active profile, name, create, delete |
| `0x04` | Macros |
| `0x05` | Power: dim timeout, power plan |
| `0x06` | Legacy LED (state, effect, brightness, blink/pulse, matrix, dock sync, knob) |
| `0x07` | Keyboard keys: key list, key mapping, debounce, snap-tap |
| `0x08` | Mouse buttons and scroll wheel (free/notched, tactile, acceleration, smart reel, masking) |
| `0x0F` | Proximity / lift-off sensor and calibration |
| `0x10` | Chroma: region list (`0x80`), effect list (`0x82`), effect (`0x03`/`0x83`), frame (`0x04`), brightness (`0x05`/`0x85`) |
| `0x11` | Thermal fan |
| `0x15` | Hapticore scroll |
| `0x16`, `0x17` | Misc / OLED |

- **observed**: The 3.0 class numbers **do not** match 2.5. For example, 2.5 puts device mode
  in class `0x00`, while 3.0 puts it in class `0x01`.
- **observed**: The 3.0 polling-rate value set is 100/125/200/250/333/500/1000 Hz (enum codes).

### Chroma frame payload (3.0, class `0x10` id `0x04`)
- **observed**: Header arg A = profile, arg B = region. Payload byte 0 = frame type:
  - type 0: no extra bytes
  - type 1 or 4: `row_start, row_end, col_start, col_end`, then RGB triplets
  - type 2: `row_start, row_end`, then RGB
  - type 3: `col_start, col_end`, then RGB
- **observed**: The full-frame path uses type 4.

## Protocol 4.0 (USB HID, `rzDevice40`)

- **observed**: Feature report **ID 2**, report length **64** including the ID. The class
  default is `deviceType = Mouse`.
- **observed**: Sent with the same Electron action as 2.5 (`hid.sendFeatureReport[Mutex]`).
  The buffer is `[0x02, …63 bytes]`. A send counts as successful when 64 bytes are written.
  The reply is read with `hid.getFeatureReport(reportId 2, length 64)`. The Electron side
  strips the leading report-ID byte, so the parser sees 63 bytes.
- **observed**: **It reuses the 2.5 command tables** (same `[size, class, id]` tuples, e.g.
  set device mode = class `0x00` id `0x04`). The OpenSynapse 2.5 command layer can be
  shared ([[Protocol/Report Format 2.5]]).

### 4.0 frame (offsets exclude the report-ID byte)

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 1 | status | `0x00` in requests. Responses use 2.5 codes (0 new, 1 busy, 2 ok, 3 fail, 4 timeout, 5 unsupported). **`0x0A` = unsolicited notification** (observed in BLE 4.0 strategy and headset parser) |
| 1 | 1 | transaction | bits 0–4: counter **1…31** (observed). Bits 5–7: **target device type** (observed) |
| 2 | 2 | ext. size? | Zero, except when the caller passes a size > 255; then it is set to the tuple's default size as big-endian (observed; **meaning unknown**) |
| 4 | 2 | data size | **big-endian** 16-bit (observed). The response parser only uses the low byte (offset 5) |
| 6 | 1 | command class | |
| 7 | 1 | command id | |
| 8 | ≤53 | arguments | |
| 61 | 1 | checksum | XOR of bytes 2…60 (observed; same XOR rule as 2.5) |
| 62 | 1 | 0x00 | |

Target device type, transaction bits 5–7 (observed):

| Value | Target |
|---|---|
| 0 | Mouse |
| 1 | Game controller 1 |
| 2 | Game controller 2 |
| 3 | Audio (headset / speaker / mic) |
| 4 | Keyboard |
| 5 | Game controller 3 |
| 6 | Game controller 4 |
| 7 | Dock / dongle |

- **inferred**: The target bits let a single dongle route commands to paired sub-devices.
- **observed**: Response validation: transaction byte, class and id must match the request,
  otherwise the request is re-sent. Optionally the read is retried on a transaction mismatch
  (`retryTransactionIdMismatch`). **The response checksum is not checked.**
- **observed**: The WebUSB fallback path in the same class still uses
  `wValue 0x0300` / length 90, copied from 2.5. This looks dead or legacy on Electron
  (inferred).
- **unknown**: Multi-frame transfers for 4.0. No offset/continuation field was found. The
  largest single argument block is 53 bytes.

### 4.0 details re-checked for the implementation (2026-11-13, [[Decisions/ADR-0015 Protocol 4.0 and 3.0 support]])

Source: `pretty/le_33_rzDevice40.js` (class `rzDevice40`: constructor defaults,
`_calculateChecksum`, `_getTransactionId`, `_createDataSend`, `_getUSBTransferInResult`,
`sendCommand`). Static reading only, no hardware.

| Item | Finding | Tag |
|---|---|---|
| Checksum | The XOR loop is the 2.5 one (indices 2..87), run over the **63-byte** buffer: indices past 62 add nothing, and it runs before bytes 61/62 are written (both still 0). Net effect: **XOR of payload bytes 2..60**, stored at 61; byte 62 = 0 | observed |
| Transaction counter | "if counter = 31 then 0; counter += 1": values **1..31**, never 0 (2.5: 0..30). The target type is ORed in as `type << 5` | observed |
| Default target | Constructor default `deviceType = Mouse` (0); only dongle-specific calls (e.g. HyperSpeed mode, THX code) pass `DockDongle` (7) | observed |
| Size bytes | Override size ≤ 255 (always for OpenSynapse, data size is a u8): bytes 2,3,4 = 0, byte 5 = size. Size > 255: bytes 2..3 = default size BE, 4..5 = size BE | observed |
| Argument copy | All bytes of the argument array are copied from offset 8 (no clamp to the size) | observed. OpenSynapse rejects sizes > 53 instead |
| Send | Same native action as 2.5 (`hid.sendFeatureReport[Mutex]`, payload tagged `protocol: "25"`), buffer `[2, …63]`; success = 64 bytes written; per-device Web Lock | observed |
| Read | `hid.getFeatureReport(reportId 2, length 64)`; reply args = `data[8 .. 8+data[5]]` | observed |
| Reply checks | Transaction byte, class, id must match, else resend (or re-read with `retryTransactionIdMismatch`, default off) | observed |
| Status handling | Identical to 2.5: 0 resend, 1 poll again (busy), 2 done, 3 resend, 4 resend (Set Device Mode timeout optionally ignored), 5 and 6..11 fail without resend | observed |
| Timing | Same constants module as 2.5 (`SLEEP_TIME_BETWEEN_*`, `MAX_RETRY_*`) | observed |
| HID collection | Picked by `claimInterface` from the device manifest (no usage check), like 2.5. **No 4.0 manifest is available**, so interface and usage of a real 4.0 device are unknown | observed (method) / unknown (values) |
| Feature report length in HID caps | Expected `FeatureReportByteLength = 64` on the collection (report ID 2 + 63) | inferred |

### Which devices speak 4.0
- **unknown / none known.** None of the ~350 fetched device manifests (`fetched/dm/`,
  `fetched/dmalt/`, `products/`) names an `rzDevice40*` class, and none has
  `hardwareEventParserType` 26/27/28 (the PROTOCOL40 parsers). The class names appear only
  in the lighting-engine loader table and the shared product bundles' chunk maps, which list
  every class and say nothing about PIDs (observed, 2026-11-13 search). 4.0 devices are
  probably among the 81 Synapse-4 products that have no fetched manifest
  ([[Devices/Device Registry]]), but nothing says which.

### 4.0 over BLE (`rzDevice40Ble`)
- **observed**: Uses the same GATT connect as 3.0. A write is a single GATT write, with no
  3.0-style header.
- **observed**: The response is matched on **byte 1** (transaction) of the notify.
  Notifies with **byte 0 = `0x0A`** are treated as unsolicited events and forwarded on
  IPC channel `rz-ble-notify-<mac>`.
- **inferred**: The BLE 4.0 frame is the USB 4.0 frame without the report-ID byte.

### 4.0 headset / audio
- **observed**: `rzDevice40Headset` and its variants use report ID 2 with the 4.0 header.
  Audio commands use class `0x18` (24) and others. See the headset event format in
  [[Protocol/Hardware Events]].

## Other enums referenced
- **observed** `deviceCommandEnum`: `RazerCommand = 8`, `RemoteCommand = 14`. They are not
  part of the HID frame. **unknown** meaning (probably mapping-engine action types).
- **observed** `usbUsageEnum`: USB HID **Consumer page** usage codes (media keys, e.g.
  `0xE2` mute, `0xCD` play/pause, `0xE9`/`0xEA` volume, `0xB5`/`0xB6` next/prev, `0x29F`
  app switch). They are used **only to decode media-key events**, not to choose an interface.
