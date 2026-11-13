# ADR-0015: Protocol 4.0 and 3.0 support

**Status:** accepted (2026-11-13). 4.0 implemented, **inferred** (no hardware). 3.0 planned, not implemented.

**Context:** The registry has 323 devices on protocol 2.5 (supported, only the Viper V3
HyperSpeed is hardware-tested), 33 PIDs on 3.0 and 170 entries/variants with
`protocol = "unknown"`. Synapse 4 also ships the 4.0 class `rzDevice40` and the BLE
classes `rzDevice30*`. There is no 4.0 or 3.0 hardware to test on. Facts and evidence:
[[Protocol/Protocol 3.0 and 4.0]], [[Protocol/Report Format 2.5]], [[Protocol/Windows Transport]].

## Decision

### Protocol 4.0: implement the frame, gate through one owner
- `opensynapse-protocol::report40` owns the 4.0 bytes: report ID 2, 63-byte payload
  (64 on the wire), status / transaction (counter 1..31, target in bits 5..7) / size at
  4..5 (BE) / class / id / 53 argument bytes / XOR checksum of 2..60 at 61. The command
  tables are the 2.5 ones, so all typed `get_*`/`set_*` builders are reused.
- `opensynapse-protocol::framing` is the single entry point: `Framing::{V25, V40(Target)}`
  and `Encoder` turn a `Request` into wire bytes; `Framing::decode` turns a reply buffer
  into the 2.5 `Report` shape. So every parser (`parse_*(report.args())`), the session's
  retry rules and all callers work unchanged. The 2.5 wire bytes are byte-for-byte the
  same as before (regression tests in `framing` and `opensynapse-hid`).
- `opensynapse-hid`: `Session::with_framing`; `Transport::open` and `HidEntry::is_control`
  take the framing from the registry protocol of the PID (`framing_for_pid`). The send
  algorithm is unchanged: Synapse's 4.0 class uses the same status handling, reply
  matching and pacing as 2.5 (observed). Differences: buffer length 64, report ID 2 in
  byte 0 of SET and GET, transaction counter 1..31, requests with data size > 53 fail
  with `ProtocolError::FrameDataSize` before anything is sent.
- Control collection for 4.0: the configured interface (0) and a 64-byte feature report.
  The usage is not checked when the OS reports the feature length, because Synapse picks
  by interface only and the 4.0 usage is unknown. Without a length (hidapi on Linux/macOS)
  the 2.5 usage pair (1/2) is required. Both inferred.
- Target: always `Target::Mouse` (the class default, observed). Keyboards, headsets or
  dongle-routed commands may need other targets (unknown).
- `opensynapse-device-registry::Protocol::framing()` / `is_supported()` are the one owner
  of "which protocols OpenSynapse speaks". The agent (`convert::is_supported`), the CLI
  (`Target::select`) and the GUI (`DeviceRow::supported`) delegate to it.
- The untested-device warning still applies: no 4.0 entry can have `tested = true` until
  someone runs it on hardware.

### Registry: no device reclassified
No fetched manifest names an `rzDevice40*` class or a PROTOCOL40 event parser, so **no
device is classified 4.0**. 4.0 support is ready but currently reaches zero registry
devices. A PID becomes 4.0 only with evidence: its device manifest
(`rzDeviceConfig.rzDeviceName = rzDevice40*` or `hardwareEventParserType` 26/27/28), or a
read-only probe on real hardware (`opensynapse hid collections` showing a 64-byte feature
collection, then a Get Firmware Version in 4.0 framing answered with status 2).

### Protocol 3.0 (BLE): not implemented; plan
3.0 runs over a vendor GATT service (`52401523-…`), not HID-over-GATT, so Windows does not
expose it as a HID collection and the existing HID path cannot reach it (inferred from the
UUID and from Synapse using a GATT client, `noble`, for it). Implementing it needs:
1. A GATT client: on Windows the WinRT `Windows.Devices.Bluetooth.GenericAttributeProfile`
   API (needs the `windows` crate with WinRT features, a new dependency to justify under
   [[Decisions/ADR-0002 Lean by design]]); BlueZ D-Bus on Linux; CoreBluetooth on macOS.
2. Discovery: paired BLE devices with VID 0x1532 or 0x068E exposing the service UUID.
3. A pure 3.0 codec in `opensynapse-protocol` (8-byte header, 24-bit LE length, no
   checksum, V2 fragmentation with repeated header, status in byte 7) with byte tests.
4. A **separate command table**: 3.0 class numbers differ from 2.5 (device mode is class
   0x01, not 0x00), so the typed builders cannot be reused as-is.
5. Hardware to test on. Until then 3.0 stays `unsupported`.

### "unknown" devices
Breakdown (primary entries, 166): 81 Synapse-4 products with no fetched manifest
(`evidence = "synapse-js"`), 48 legacy Synapse-2-only products, 28 with audio / IoT /
camera classes that use vendor transports (`Audio*`, `rzDeviceIoT`, `rzDeviceElsa`), 8
Synapse-4 manifests without a driver class, 1 inferred. Plus 63 variant PIDs without a
known protocol. To classify one, any of: its `DeviceManifest_<pid>_<edition>.json`
(class name or event-parser type), an OpenRazer entry for the PID (2.5), or a read-only
HID-caps probe on the device (91-byte feature on usage 1/2 = 2.5 shape; 64-byte = 4.0
shape) confirmed by a Get Firmware Version answer. None of these was available for the
unknown entries in this session, so none was reclassified.

## Consequences
- 4.0 devices will work as soon as the registry names them, with the untested warning.
- Risk: every 4.0 fact is from static reading of shipped JS; the first real 4.0 device may
  show a different interface, usage, target or size rule. The first 4.0 test must be
  read-only (`info`), never a write.
- The `Framing` seam is where a future 3.0 transport would plug in only if it stays
  request/response with the 2.5 command tables; it does not, so 3.0 will need its own
  session type.
