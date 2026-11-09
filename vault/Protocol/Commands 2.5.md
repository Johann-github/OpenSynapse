---
tags: [protocol, p25, commands]
status: draft
source: Synapse 4 background-manager index-*.js (base class rzDevice25) + rzDevice25*.js
---
# Protocol 2.5 — command table

Framing: [[Protocol/Report Format 2.5]]. Device: [[Devices/Viper V3 HyperSpeed]]. Log: [[RE-Log/2026-11-09 Protocol 2.5]].

Notation: `class/id`, args = bytes from offset 8. `u16` = big-endian (hi, lo) — observed. Get = id | 0x80. "size" = default data_size (byte 5). Tags: observed / inferred / unknown.

Note: `deviceCommandEnum` is **not** the command table — it only holds two IPC category values (8, 14). The real table is ~430 `[size, class, id]` constants in the shared `index` module (observed).

## Mouse-relevant commands (detailed)

### Device info / system (class 0x00)

| Cmd | Dir | size | Args (request → response) | Tag |
|---|---|---|---|---|
| 00/81 Firmware Version | get | 2 | → `[major, minor]` | observed |
| 00/87 Extended Firmware Version | get | 4 | → `[major, minor, internal, reserved]` | observed |
| 00/83 Protocol Version | get | 2 | → `[major, minor]` | observed |
| 00/82 Serial Number | get | 22 | → 22 bytes printable ASCII | observed |
| 00/84 · 00/04 Device Mode | get/set | 2 | `[mode, param]`; 0 normal, 1 bootloader, 2 testing, 3 driver, 4 download, 5 pairing, 6 standby, 8 RF test, 9 retail | observed |
| 00/86 · 00/06 Edition Information | get/set | 3 | `[keyboardLayout (0 = non-keyboard), edition, firmwareId]`; Synapse uses the get as "is powered on" probe (3 tries, 100 ms) | observed |
| 00/0B Device Reset | set | 1 | `[resetType]`, values unknown | observed / unknown |
| 00/B3 · 00/33 Ambidextrous | get/set | 1 | `[leftHand]` | observed |
| 00/B5 Device HW Info | get | 80 | `[type, count, count × u16 …]` | observed |

### Polling rate

| Cmd | Dir | size | Args | Tag |
|---|---|---|---|---|
| 00/85 · 00/05 Device Polling Period | get/set | 1 | `[code]`; 1=1000, 2=500, 3=333, 4=250, 5=200, 8=125, 10=100 Hz (1000/code) | observed |
| 00/8E · 00/0E Device Profile Polling Period | get/set | 2 | get req `[profile]`; `[profile, code]` | observed |
| 00/C0 · 00/40 USB High Speed Polling (HyperPolling) | get/set | 2 | get req `[profile]`; `[profile, code]`; bitmask 0x01=8000, 0x02=4000, 0x04=2000, 0x08=1000, 0x10=500, 0x20=250, 0x40=125 Hz | observed |

Which polling command the Viper supports is chosen by the per-product UI (not cached) — unknown. Plausibly 00/40 for HyperSpeed devices (inferred).

### DPI (class 0x04)

| Cmd | Dir | size | Args | Tag |
|---|---|---|---|---|
| 04/85 Active DPI | get | 7 | req `[storage]` → `[storage, X u16, Y u16, Z u16]` | observed |
| 04/05 Active DPI | set | 7 | `[storage, X u16, Y u16, Z u16]` (Z = 0) | observed |
| 04/86 DPI stages | get | 80 | req `[storage]` → `[storage, activeStage, n, n × (stageId, X u16, Y u16, Z u16)]` | observed |
| 04/06 DPI stages | set | `7·n+3` | same layout | observed |

- Byte 0: OpenRazer uses 0x00 = volatile, 0x01 = persistent (inferred); Synapse callers not cached (unknown).
- Separate X/Y DPI supported by layout (observed). Ranges not in this code (unknown).

### Power / battery (class 0x07)

| Cmd | Dir | size | Args | Tag |
|---|---|---|---|---|
| 07/80 Battery Level | get | 2 | req `[batteryId]` → `[batteryId, level 0–255]`; % = floor(level/255·100) | observed |
| 07/84 Charging Status | get | 2 | → `[batteryId, status]`; 0 not charging, 1 charging, 2 failed, 3 foreign object, 4 fast, 5 full, 6 limit reached | observed |
| 07/81 · 07/01 Low-battery threshold | get/set | 1 | `[0–255]`; % → ceil(p/100·255) | observed |
| 07/83 · 07/03 Time to sleep on battery | get/set | 2 | `[time u16]`; unit unknown (seconds inferred) | observed (layout) |
| 07/88 · 07/08 Auto Sleep Mode | get/set | 2 | `[profile, enabled]` | observed |
| 07/8B · 07/0B Time To Dim | get/set | 5 | `[profile, enabled, time u16, percentage]` | observed |
| 07/82 · 07/02 LED max brightness on battery | get/set | 1 | `[brightness]` | observed |
| 07/95 Low Power Mode State | get | 1 | `[state]` | observed |
| 07/96 Dongle Connection Quality | get | 1 | `[quality]`; 1 red … 4 green (inferred) | observed |
| 07/A1 Keep Device On | get-style | 0 | — | observed |

### Wireless / dongle (class 0x00)

| Cmd | Dir | size | Args | Tag |
|---|---|---|---|---|
| 00/B8 Wireless Connection Status | get | 1 | → `[connected]` | observed |
| 00/BF Multi-device wireless status V2 | get | 80 | → `[count, count × (status, PID u16)]` | observed |
| 00/C5 Primary Device | get | 3 | → `[type, PID u16]`; 1 mouse, 2 keyboard, 3 headset, 4–7 controllers | observed |
| 00/41 Pairing Mode | set | 3 | `[mode (1 start, 4 cancel), PID u16]`; DualLinkMouse sends with TID 0x80 bit | observed |
| 00/42 Unpair | set | 2 | `[PID u16]` | observed |
| 00/46 Device Scan | set | 1 | `[1 start / 4 cancel]`; TID 0x80 bit | observed |
| 00/C7 · 00/47 Active wireless connection | get/set | 2 | `[type (1 2.4 GHz, 2 BLE PC, 3 BLE mobile), status]` | observed |

### Buttons / remapping (class 0x02)

| Cmd | Dir | size | Args | Tag |
|---|---|---|---|---|
| 02/84 Button ID list | get | 80 | → `[total, ids…]` | observed |
| 02/0C Single Button Assignment | set | 80 | `[profile, buttonId, mode, functionId, dataLen, data…]` | observed |
| 02/8C Single Button Assignment | get | 80 | req `[profile, buttonId, mode]` → same as set | observed |
| 02/8E Button Assignment List | get | 80 | req `[profile, mode, idOffset]` | observed |
| 02/A5 · 02/25 Button Mapping Mode | get/set | 2 | `[profile, mode]` | observed |
| 02/94 · 02/14 Scroll free/notched | get/set | 2 | `[profile, 0 notched / 1 free]` | observed |
| 02/B0 · 02/30 Scroll tactile/free/micro | get/set | 2 | `[profile, 0/1/2]` | observed |
| 02/82 · 02/02 Debounce Time | get/set | 4 | `[makeDebounce, makeMask, breakDebounce, breakMask]` ms | observed |

- mode: 0 normal, 1 HyperShift (observed).
- functionId: 0 off, 1 mouse button, 2 key, 3/4/5/15 macro types, 6 DPI, 7 profile, 8 lighting, 9 power keys, 10 media keys, 11 double-click, 12 mode key, 13 turbo key, 14 turbo button, 16 controller, 17 Razer key, 18 scroll mode, 21 Win8 shortcut (observed).
- buttonId: 1 left, 2 right, 3 wheel click, 4–8 buttons 4–8, 9 wheel up, 10 wheel down, 11 DPI up, 12 DPI down, 13 user select, 14 profile switch, 15 sniper, 16–31 macro 1–16, 52/53 tilt L/R, 57 HyperShift, 104/105 scroll L/R, 106 free/notch switch, 121 rapid fire (observed).
- Data bytes per functionId are built by the per-product UI — unknown.

### Profiles (class 0x05)

| Cmd | Dir | size | Args | Tag |
|---|---|---|---|---|
| 05/80 Number of Profiles | get | 1 | → `[n]` | observed |
| 05/8A Max Profiles | get | 1 | → `[n]` | observed |
| 05/81 Profile ID list | get | 80 | → `[count, ids…]` | observed |
| 05/02 Create / 05/03 Delete | set | 1 | `[profileId]` | observed |
| 05/84 · 05/04 Active Profile | get/set | 1 | `[profileId]` | observed |
| 05/88 · 05/08 Profile Name with offset | get/set | 80 | `[profileId, offset u16, size u16, bytes…]` | observed |
| 05/0B Profile Reset | set | 1 | `[profileId]` | observed |
| 05/8C · 05/0C Profile Color | get/set | 4 | `[profileId, R, G, B]` | observed |

### Lighting — new style (class 0x0F)

| Cmd | Dir | size | Args | Tag |
|---|---|---|---|---|
| 0F/80 Region ID list | get | 80 | 5 bytes per region `[regionId, updateRate, dataType, arrayX, arrayY]` | observed |
| 0F/81 Effect ID list | get | 80 | req `[regionId]` → `[regionId, effectIds…]` | observed |
| 0F/02 · 0F/82 Effect | set/get | `6+3n` | `[profile, region, effectId, flags, rate, nColors, n × RGB]` | observed |
| 0F/04 · 0F/84 Brightness | set/get | 3 | `[profile, region, 0–255]` | observed |
| 0F/03 Frame | set | `5+3n` | `[profile, region, row, startCol, endCol, RGB…]`, no reply wait | observed |

effectId: 0 off, 1 static, 2 breathing, 3 spectrum, 4 wave, 5 reactive, 6 ripple, 7 starlight, 8 custom frame, 9 fire, 10 audio meter, 11 immersive, 12 wheel (observed). Region: 0 all, 1 wheel, 2 DPI LED, 3 battery, 4 logo, 5 backlight, 10 underglow, 11 side stripe (observed).

### Lighting — legacy (class 0x03)

| Cmd | Args | Tag |
|---|---|---|
| 03/00 · 03/80 LED State | `[ledClass, ledId, on]` | observed |
| 03/01 · 03/81 LED RGB | `[ledClass, ledId, R, G, B]` | observed |
| 03/02 · 03/82 LED Effect | `[ledClass, ledId, effect]` | observed |
| 03/03 · 03/83 LED Brightness | `[ledClass, ledId, 0–255]` | observed |
| 03/0A Matrix effect | `[effect, …]`; 0 off, 1 wave, 2 reactive, 3 breathing, 4 spectrum, 5 custom, 6 static | observed |
| 03/0B Matrix frame row | `[frameId, row, start, end, RGB…]`, size `4+3n` | observed |

## Viper V3 HyperSpeed specifics
From product manifest 184 (observed): driver class `rzDevice25DualLinkMouse`, interface 0, dual-link dongle, timing 25 ms, **no Chroma lighting**, event parser = Protocol25, 7 mouse inputs. Likely command set (inferred): 00/81, 00/82, 00/84/04, 04/85/05, 04/86/06, polling 00/40 or 00/05, 07/80, 07/84, 07/03/83, 07/01/81, 02/0C/8C, 00/BF/C5.

## Appendix — other command classes (names + IDs, observed)
- **0x00** also: 08/88 game mode, 0E/8E profile polling, 91–93 dock fw/serial, 37/B7 external power, B9 side pad, BB dock charging, C3 Falcon protocol version, C4/44 HW auto detect, D5/55 auto pairing, D7 power button, 58/D8 wired multi-device, 5A–5D game/tournament mode.
- **0x02** keyboard/analog: 0D/8D key assignment, 8F key list, 12/92 analog key, 19–1A actuation/rapid trigger, 1B mod-tap, 1E multi key, 1F–20 analog adjust, 21 kill switch, 22–23 scroll accel/smart reel, 24 layout switch, 27/2C snap tap, 2D–2E interruptible macro/keystroke, 31 dual keypress priority, B2/33–37 keymaps, 38–39 analog joystick.
- **0x03** legacy LED: 04 blinking, 05 pulsating, 06 SW param, 07 transition, 08 max value, 89 LED ID list, 0C lighting bar, 0D extended matrix, 0E color table, 0F dock sync, 10 charging effect, 12–13 knob, 14 meter, 20 GPU detect.
- **0x06** macros (on-device): 80 count, 02 create, 03 delete, 85 data, 08/88 allocate/size, 09/89 memory data, 0A/8A memory mgmt, 8B ID list, 0C/8C name, 8E storage info.
- **0x08** audio: EQ preset/setting, level, volume, bass, mute, source, I/O status, Dolby, 3D, night mode, peak, HPF, mic volume, dial, combo.
- **0x0B** proximity/lift-off sensor: list, calibration, threshold, state, configuration, accuracy, lift settings, acceleration curve, angle tune (IDs 00–14 / 80–94).
- **0x0D** thermal/fan (laptops), VGA, mini-LED.
- **0x0F** also: 05 calibration, 06 Fn overlay, 07 new frame, 08 stripe LED number, 10 force control, 13 tap, 14 quick switch, 17 firmware effects, 18 startup animation.
- **0x15** HyperCore haptic scroll, **0x16** Interhaptics, **0x17** OLED, **0x18** audio DND (set 0x15 / get 0x85), **0xFE** analog calibration (test mode).
