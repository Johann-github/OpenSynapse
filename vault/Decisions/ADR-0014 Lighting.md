# ADR-0014: Lighting (RGB) for protocol-2.5 devices

**Status:** accepted (2026-11-13), **untested on hardware**

**Context:** Users want to set colours on Razer mice and keyboards. The only test device
(Viper V3 HyperSpeed) has no lighting, so every lighting write is unverified. Facts and
evidence: [[Protocol/Lighting 2.5]]. Constraints: [[Decisions/ADR-0001 Clean-room reverse engineering]],
[[Decisions/ADR-0002 Lean by design]] (no new crates, no software effect loop, idle CPU ~0 %).

## Decisions

1. **Hardware effects only.** OpenSynapse sends one effect command and the device runs the
   effect itself. No per-frame streaming (0F/03, 03/0B), no software animation, no Chroma
   SDK: a frame loop would break the idle-CPU budget.
2. **First effect set:** off, static colour, breathing (one or two colours), spectrum
   cycling, wave (direction 1/2), plus brightness. Reactive, ripple, starlight, fire,
   audio meter, custom frames are left out (need speed/frame data or a host loop).
3. **Family per device from the registry** (single owner: `opensynapse-device-registry`,
   `Device::lighting_family()`): the Synapse driver class decides (table in
   [[Protocol/Lighting 2.5]]).
   - `extended` (class 0x0F) for `rzDevice25` and the classes that inherit its lighting
     methods.
   - `standard` (class 0x03, 03/0A matrix effect) for the `LedMatrix` / `Legacy` classes.
   - No family (lighting refused) when the registry flags the product non-Chroma, has no
     LED matrix, or the class is anything else. The per-LED family
     (`rzDevice25LegacyHardwareEffect`) is not implemented: its only product is non-Chroma.
   - Brightness is refused for the `*NoBrightness` classes (inferred from the name).
4. **Zones** come from the registry (`lighting.zones`, region IDs from the device
   manifests). Without a zone list the default is 0 ("all"), Synapse's own default for
   effect writes. Reading zone 0 back is not known to work, so read-back uses the requested
   zone and a failing read means no write (the GUI and the agent never write blind).
5. **Storage.** Extended family: byte 0 is 0 = volatile (default, same as `dpi set`), 1 =
   saved (`--persistent` in the CLI, "Save on device" in the GUI). A volatile mistake is
   undone by unplugging the device: restore-safe. Standard family has no storage byte.
6. **Bytes only in `opensynapse-protocol`** (`lighting::Mode`, `extended`, `standard`
   modules). Brightness is in percent in the UI; the protocol crate converts with
   Synapse's rounding (0x0F: floor out, ceil back; 0x03: ceil out, floor back).
7. **Read-back and restore.** Extended: 0F/82 + 0F/84 exist, so CLI, GUI and agent read
   the old effect and brightness first, write, read back and offer the restore (CLI prints
   the restore command). Standard: 03/0A has no GET; the CLI writes it only with `--apply`
   like every write, says that no read-back or restore is possible, and the GUI does not
   offer it (shown as "CLI only").
8. **Untested warning.** No lighting device is `tested = true`, so the existing
   untested-device warning (`warn_if_untested`, GUI warning) covers every lighting write;
   no extra lighting text.
9. **GUI:** a Lighting section (effect, colour picker, second colour, brightness slider,
   zone, "Save on device", Apply) only for extended-family devices, default colour the
   app accent green (`opensynapse_icon::ACCENT`). Writes go through the agent's `Write`
   request like DPI (new `WireSetting::Lighting`, IPC protocol version 4).

## Follow-ups
- **Profiles:** an optional `[lighting]` table in the TOML profile (per-app colours) needs
  the core plan, the device-side apply with restore file, and export. Not simple enough for
  this slice; parked in [[Backlog]].
- Snapshot read of the current lighting for the GUI panel (now the GUI starts from the
  accent green, not from the device state).
- Hardware test on any extended-family mouse (e.g. Basilisk V3) and one `LedMatrix`
  keyboard: confirm byte 0 meaning, wave direction, zone-0 read.
- Protocol 3.0 / 4.0 lighting.
