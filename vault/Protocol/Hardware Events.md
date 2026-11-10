# Hardware events (device → host)

Unsolicited messages a device sends on its own, for example: DPI button pressed, profile
switched, battery changed, receiver connected or disconnected. Command/response traffic is
in [[Protocol/Report Format 2.5]] and [[Protocol/Protocol 3.0 and 4.0]].
Evidence: [[RE-Log/2026-11-09 Protocol 3.0 4.0 and events]].

Each claim has a tag: **observed** (read directly in shipped code), **inferred**
(follows from the code but is not stated there), **unknown**.

## Which parser a device uses

**observed**: The device manifest field `hardwareEventParserType` (numeric) picks the
parser. When the field is missing, PROTOCOL25 is the default.

| Value | Parser | Notes |
|---|---|---|
| 1 | PROTOCOL25 | Most mice and keyboards. **Viper V3 HyperSpeed (PID 184) = 1** |
| 6 | PROTOCOL30 | Same as 25, but strips one leading byte first |
| 9 / 17 / 18 / 19 | 25 Glitter / Esther / OLED / 30 OLED | Variants |
| 20 / 23 / 24 / 25 / 30 | 25 controller, 25 controller 8 kHz, PlayStation, PS arcade, controller notification | Controllers |
| 26 / 27 / 28 | PROTOCOL40, 40 earbuds, 40 headset | |
| 29 | 25 keypad | |
| 2, 3, 5, 7, 8, 12, 13, 15, 16, 21, 22 | Audio parsers (Avnera, April, NXP, Mixer, Mxic…) | Out of scope for mice |
| 4, 10, 11, 14 | KateT2, PWM PC fan, Elsa, Layla | |

## Delivery paths (how the bytes reach Synapse)

1. **observed, main path for 2.5 devices**: The native mapping engine
   (`RzMappingEngine.addUsbDevice` + `registerHardwareEvent`, plus a Razer filter driver
   unless `skipFilterDriver` is set) pushes `hardwareevent` IPC messages to the web app.
   **unknown**: Which HID collection the filter driver reads from. It sits inside native DLLs
   that were not analysed.
2. **observed, direct HID path** (`hid.registerHidDeviceEvent`, Electron
   `modules/hidHardwareEvents`): node-rz-hid opens the single HID collection that matches
   VID, PID, container id **and** `hidConfig.usagePage` + `hidConfig.usageId`. Each input
   report is forwarded as an `hidhardwareevent` IPC message. When `hidConfig.appendReportId`
   is set, the report ID (`hidConfig.reportId`, default **5**) is put back in front of the
   data, because the OS strips it. The device middleware turns this on with
   `useHidHwEvents.config.hidConfig`.
   **unknown**: The concrete usagePage/usage values. They live in per-product middleware
   configs that were not captured.
3. **observed**: Batched path (`onhardwareeventarray`), used only for the 8 kHz controller
   parser.
4. **observed**: BLE 4.0. GATT notifies with byte 0 = `0x0A` are forwarded on
   `rz-ble-notify-<mac>`.

- **inferred (important for OpenSynapse)**: The default reportId 5 is the same number as
  record type NOTIFICATION_EVENTS (5). So the "record id" at byte 0 of an event **is the HID
  input report ID** on a vendor collection. A Rust implementation should open the vendor
  input collection with hidapi and dispatch on report ID. Verify with a capture on the Viper
  V3 HS before relying on this.

## Generic record layout (PROTOCOL25)

**observed**: Byte 0 = record id (report ID), followed by the record body.

| Record id | Name | Body |
|---|---|---|
| 0 | not supported | — |
| 1 | keys extended buffer | not parsed |
| 2 | media keys | Consumer usage = byte1 \| byte2<<8 (16-bit, LE), see `usbUsageEnum` |
| 3 | power keys | byte1: 0 power, 1 sleep, 2 wake |
| 4 | Razer keys | bytes 1… = list of Razer key codes |
| 5 | **notification event** | byte1 = event id, bytes 2… = event payload (table below) |
| 6 / 7 | analog keyboard ADC data / key | not parsed in the 25 parser |
| 8 | dongle secondary device Razer keys | as record 4 |
| 9 | **dongle secondary device notification** | as record 5 (a second device on the same dongle) |
| 10 | dongle secondary device mouse events | not parsed |
| 77 (`0x4D`) | calibration mouse redirect | byte1 button id, byte2 state |
| 90 (`0x5A`) | calibration Z/Q data | byte1 z, byte2 q |
| 97, 103, 104 | audio haptic intensity, keymap cycle button, profile cycle button | not parsed here |
| 240 (`0xF0`) | controller sync (synthetic, controller parsers) | |

## Notification events (record 5 / 9): mouse-relevant

**observed**: Offsets count from byte 0 (the record id). The event id is always byte 1.
Multi-byte values are **big-endian** (high byte first).

| Event id | Name | Payload |
|---|---|---|
| 1 (`0x01`) | profile change | b2 = new profile |
| 2 (`0x02`) | **DPI change** | X = b2..b3 (BE16), Y = b4..b5, Z = b6..b7 |
| 3 / 4 | detachment / attachment | none |
| 5 / 6 | lifted / put down (lift-off) | none |
| 8 | brightness change | b2 |
| 9 | **wireless change** | b2 state (1 reserved, 2 disconnect, 3 connect), b3 device type (1 mouse, 2 keyboard, 3 headset, 4–7 controller, 8 pure-chroma) |
| 10 | calibration change | b2 (0 start, 1 end, 2 halted, 3 reset, 4 normal speed, 5 too fast, 6 cancelled) |
| 12 | external power source change | b2: 0 removed, 1 connected |
| 13 | device hardware change | b2..b5 = four slots (0 removed, 1 connected) |
| 14 | mouse keypad module change | b2 side-plate type (0 none, 1 = 2-key, 2 = 7-key, 3 = 12-key, 4 = 6-key) |
| 24 | LED effect/colour changed | b2 effect, b3 flags, b4 rate, b5 n colours, then n×RGB |
| 49 (`0x31`) | **battery change** | b2 level 0–255 (Synapse shows ⌊b2/255·100⌋ %), b3 charging (0 not charging/full, 1 charging, 2 fault), b4 external power (0/1), b5 device type (as event 9) |
| 50 | HW detection done | none |
| 53 (`0x35`) | wireless change V2 | 2.5/4.0 parser: b2 state, b3 type, b4..b5 PID (BE). **3.0 parser swaps them**: b2 link type (0 n/a, 1 2.4 GHz, 2 BLE-PC, 3 BLE-mobile), b3 state (0/1) |
| 54 | pair status | b2: 0 n/a, 1 started, 2 success, 3 timeout, 4 cancelled |
| 55 | scan status | b2 status (1 start, 2 end, 4 cancel), b3 count, then 3 × {PID BE16, layout, edition} from b4 (3.0: PID BE16 + edition BE16) |
| 56 (`0x38`) | active connection change | b2 link type (0 n/a, 1 2.4 GHz, 2 BLE-PC, 3 BLE-mobile), b3 connected |
| 57 | scroll mode change | b2: 0 notched, 1 free spin, 2 micro-tactile |
| 58 | Hapticore scroll stage | b2 active stage, b3 stage index |
| 59 | external power V2 | b2 state, b3..b4 PID, b5..b6 edition |
| 65 (`0x41`) | battery fully charged | none (25 parser also has a variant with b2 = state) |
| 70 | button change | b2 type (1 fan button, 2 chroma button), b3 mode (1 on, 2 off, 3 cycle) |
| 71 | sensor temperature | b2 |
| 86 (`0x56`) | **dongle connection quality** | b2: 1 red, 2 orange, 3 yellow, 4 green |
| 88 (`0x58`) | **polling rate change** (25 parser only) | b2 bitmask: 1 = 8000, 2 = 4000, 4 = 2000, 8 = 1000, 16 = 500, 32 = 250, 64 = 125 Hz |
| 89 | keyboard mode slider | b2 |

Other ids that are defined but not mouse-relevant (observed): audio volume / mute / input /
EQ / Dolby / ANC (15–22, 33, 68, 87), 3-D mode 26, camera tracking 29, THX 32, fan
48/64, power wattage/security 51/52, analog key mode 66, chroma tap 67, OLED 81–85, haptics
112–114. **observed**: 85 is used twice (OLED low-power and Valkyrie sidetone), and 32
is used twice (THX and haptic gain). The parser type decides which meaning applies.

- **inferred**: Of all the notifications, Synapse uses the wireless change event
  (type mouse/keyboard, state 3 = connected) as the "device powered on" signal for 4.0
  devices (`getPowerOnStatusFromHardwareEvent`).
- **inferred**: OpenSynapse's DPI-button sync should listen for event 2 and update the
  stored DPI stage. No round-trip query is needed.

## PROTOCOL30 differences
- **observed**: The parser drops byte 0 first. After that the layout is the 2.5 one.
  So a 3.0 event buffer has one extra leading byte (inferred: a report ID / BLE framing byte).
- **observed**: Only a subset of events is decoded (no audio, no polling-rate, no
  dongle-quality). Wireless change V2 uses the swapped byte order shown above.
- **observed**: Product id `3885` (`0x0F2D`) re-labels event 58 as an LED effect change
  (region, effect, direction, rate, n colours, RGB…).

## PROTOCOL40 differences
- **observed, background-manager copy**: Record id is read from **byte 1**. The
  notification parser also reads the event id from byte 1. Both cannot be right at the
  same time.
- **observed, lighting-engine copy (newer)**: Record id is read from byte 0, which matches
  the 25 parser.
- **inferred**: The background-manager copy has a bug or expects a different framing. For
  OpenSynapse, use the 25 layout and confirm with a capture. **unknown** which framing
  4.0 mice really send.
- **observed, 4.0 earbuds**: Record 5 only. Event 49 = left battery b2, right battery b3.
  Event 56 = b2 type (0 headset, 1 earbuds), b3 status, b4 BT status.
- **observed, 4.0 headset** (`rzHardwareEventsProtocol40ParserHeadset`): Uses the 4.0
  **command frame** format. Byte 0 = report ID 2, byte 1 = `0x0A` (notification marker in
  the status slot). From byte 6 on: payload length, class, id, then payload. The event type
  comes from (class, id) (e.g. class `0x18` audio). Events: wireless status, battery,
  charging, mute, mic mute, ANC, EQ preset…

## 8 kHz controller (PROTOCOL25_CONTROLLER_8K)
- **observed**: Two report shapes, chosen by length:
  - **15-byte analog report** (byte 0 = 0):
    - left stick X/Y = b1..b2 / b3..b4 (LE16, centre 0x8000)
    - right stick X/Y = b5..b6 / b7..b8
    - triggers = b9..b10 combined (LE16 minus 0x8000; positive = left trigger, negative = right trigger)
    - buttons bitmask b11 (A 1, B 2, X 4, Y 8, LB 16, RB 32)
    - d-pad b12 (0 none, 4 up, 8 up-right … 32 up-left, steps of 4)
  - **64-byte sync report**: b9 = sync event id, data from b10:
    - 1–4 chroma effect / speed / brightness / colour (effect b10, flags b11, rate b12, RGB b13–15; brightness b16)
    - 5 haptics b10
    - 6 M-button mapping (button b10, firmware id b11)
    - 7 profile b10
    - 8 battery (b10 level 0–255, b11 charging)
    - 9 wireless b10
    - 10 RF connect, 13 SOCD, 14 restore defaults
- **observed**: Analog reports arrive in batches. A report is dropped when the button byte
  has not changed, except for the first and last report of a batch.

## Open points
- unknown: HID interface / usage page / usage of the event collection on the Viper V3 HS.
  Needs a hidapi enumerate plus capture.
- unknown: Whether the Razer filter driver changes the reports before the mapping engine
  forwards them.
- unknown: 4.0 mouse event framing (see above).

## Viper V3 HyperSpeed (hardware, 2026-11-10)
- Events arrive on **MI_01 COL06** (usage page 0x0001, usage 0x0000, input 16 bytes), in device mode Normal (observed).
- DPI button press → `05 02 XX XX YY YY` (DPI X/Y, u16 big-endian) (observed).
- The agent listens there by default. → [[RE-Log/2026-11-10 Hardware events capture]]
