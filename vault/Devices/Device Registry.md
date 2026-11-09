---
tags: [devices, moc]
---
# Device registry

Machine-readable source: `data/devices/*.toml` (loaded by `opensynapse-device-registry`).
Schema: `data/devices/SCHEMA.md`. Build log: [[RE-Log/2026-11-09 Device database]].

- [[Devices/Viper V3 HyperSpeed]]: dev hardware, PID `0x00B8`
- Protocol background: [[Protocol/Overview]], [[Protocol/Report Format 2.5]], [[Protocol/Protocol 3.0 and 4.0]]

Status: 🟢 first full extraction (2026-11-09). 448 products, 582 distinct USB PIDs including receiver and Bluetooth variants.

## Files

| File | Kinds | Entries |
|---|---|---|
| `data/devices/mice.toml` | mouse | 114 |
| `data/devices/keyboards.toml` | keyboard (145, incl. laptop keyboards), keypad (8) | 153 |
| `data/devices/headsets.toml` | headset (72), speaker (11) | 83 |
| `data/devices/other.toml` | mousepad 8, dock 14, controller 13, other 63 | 98 |

## Counts

| Slice | Entries | Protocol 2.5 | 3.0 | unknown |
|---|---|---|---|---|
| Synapse 4 supported list | 331 | 221 | 1 | 109 |
| Synapse 2 legacy list only (`synapse4 = false`) | 107 | 59 | 0 | 48 |
| Xbox / Windows-10 lists and OpenRazer-only | 10 | 1 | 0 | 9 |
| **Total (primary PIDs)** | **448** | **281** | **1** | **166** |
| Variant PIDs (receiver / BLE) | 137 | 42 | 32 | 63 |

By kind (all): keyboard 145, mouse 114, headset 72, other 63, dock 14, controller 13,
speaker 11, keypad 8, mousepad 8.

By evidence: `synapse-manifest` 354, `synapse-js` 92, `openrazer` 1, `inferred` 1.
Capability coverage: DPI range on 76 mice, polling-rate list on 66, battery flag on 34,
LED matrix size on 206, non-Chroma flag on 87, OpenRazer transaction id on 205.

No product in the data uses protocol 4.0 yet (unknown). No fetched manifest names an
`rzDevice40*` class, although Synapse ships those classes; see [[Protocol/Protocol 3.0 and 4.0]].

## How Synapse maps product id, PID and protocol

1. **Product id = USB PID** (observed). The Synapse "product id" is the USB product id
   written in decimal under VID `0x1532` (5426). Examples: 184 = `0x00B8`. The driver package for 184
   is also named after `PID00b8`. Old and new lists, Xbox lists and OpenRazer PIDs all agree.
2. **Supported set** (observed). The dashboard list `AvailableDevices` holds about 335 entries
   `{productId, dongleId?, bleId?, isNotChromaDevice?}`. `productId` is the wired PID, or
   the receiver PID for receiver-only products. `dongleId` and `bleId` are the alternate PIDs
   of the same product.
3. **Per-PID device manifest** (observed). The lighting engine loads
   `DeviceManifest_<pid>_<edition>.json`. It holds `category`, the product name, the
   wired/dongle/usb PIDs, `isDualLink`, battery flags, the LED matrix, and
   `rzDeviceConfig.rzDeviceName`, which is the **driver class** Synapse instantiates. It also
   holds `hardwareEventParserType`, an enum that selects the input-event parser.
4. **Protocol = driver-class family** (observed for the naming, inferred for the
   class-to-family mapping). `rzDevice25*` speaks the 90-byte 2.5 feature report, `rzDevice30*`
   speaks 3.0 (BLE GATT), and `rzDevice40*` speaks 4.0. The parser enum agrees: values 1, 9, 17, 18, 20, 23, 24, 25 and 29
   are the "PROTOCOL25*" parsers, 6 and 19 are "PROTOCOL30*", and 26, 27 and 28 are "PROTOCOL40*". Audio, camera and IoT
   classes (`Audio*`, `rzCam*`, `rzDeviceIoT`, `rzDeviceElsa`) use their own transports, so
   their protocol is recorded as `unknown`.
5. **Protocol is per PID, not per product** (observed). The Bluetooth PID of a product
   uses `rzDevice30` (3.0) while its wired and receiver PIDs use `rzDevice25*`. Examples are
   BlackWidow V3 Pro (`0x025A` wired 2.5, `0x025C` dongle 2.5, `0x025B` BLE 3.0) and
   Basilisk V3 Pro (`0x00AA`/`0x00AB` 2.5, `0x00AC` 3.0). This is why the TOML records
   `[[device.variants]]` with their own `protocol`.
6. A second, independent table in the background manager (on-the-fly-macro module) maps
   102 PIDs to `{type, protocol name, class}`. It is consistent with the manifests and it
   names `razerProtocol30` only for BLE keyboard PIDs (observed).
7. Per-product UI bundles (`products/<pid>/mw/`) carry the user-facing ranges: min, max and
   step DPI, the polling-rate list, `dongleId`/`bleId`, battery events and onboard-profile
   slots (observed).

## Detection recipe for OpenSynapse (inferred)
`(vid, pid)` → look up primary `pid` or any `variants.pid`. Take the protocol from that
exact PID (variant overrides device). Use `unknown` to mean "do not talk to it yet".

## Known gaps
- 81 Synapse-4 products have no device manifest (mostly 2024+ mice, controllers, audio and
  cameras), so their protocol is `unknown` unless OpenRazer lists the PID. 59 variant PIDs
  have no protocol for the same reason.
- Lighting zone names are not in the data. Only the matrix size and LED count are recorded.
- `transaction_id` comes from OpenRazer. Synapse uses a rolling counter (see
  [[Protocol/Report Format 2.5]]).
- Kinds for legacy and OpenRazer-only entries are derived from the product name (inferred).
