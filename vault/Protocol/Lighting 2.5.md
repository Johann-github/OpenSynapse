---
tags: [protocol, p25, lighting]
status: draft
source: Synapse 4 background-manager (index-*.js lighting helpers, rzDevice25LedMatrix, rzDevice25Legacy*, rzDevice25ChromaEffect, otfm-*.js), Synapse device manifests (ledConfig), OpenRazer (cross-reference only)
---
# Protocol 2.5 — lighting

Back to [[Protocol/Commands 2.5]] (command IDs) and [[Protocol/Report Format 2.5]] (framing).
Decision: [[Decisions/ADR-0014 Lighting]]. Log: [[RE-Log/2026-11-13 Lighting]].

**Nothing in this note is hardware-tested.** The only test device (Viper V3 HyperSpeed) has
no lighting. Tags: **observed** = read in Synapse 4 code or manifests; **inferred** =
deduced (or taken from OpenRazer as a cross-reference); **unknown**.

## Three command families

Synapse picks the lighting commands by the device-driver class (`synapse_class` in
`data/devices`), not by device kind.

| Family (OpenSynapse name) | Commands | Synapse classes | Tag |
|---|---|---|---|
| **extended** | 0F/02 effect, 0F/82 get effect, 0F/04 · 0F/84 brightness, 0F/03 frame | base `rzDevice25` and classes that do not override its lighting methods: `rzDevice25DualLinkMouse`, `rzDevice25DualLinkKeyboard`, `rzDevice25WirelessKeyboard`, `rzDevice25MikaWirelessKeyboard`, `rzDevice25ChromaEffect` (overrides only the frame path), `rzDevice25NoSerial`, `rzDevice25NoBrightness` | observed (base methods, no overrides in the subclass files) / inferred (`NoSerial`, `NoBrightness`: subclass source not in the RE material, name only) |
| **standard** (legacy matrix) | 03/0A matrix effect (no GET), 03/0B frame row, 03/03 · 03/83 LED brightness, 03/89 LED ID list | `rzDevice25LedMatrix`, `rzDevice25LedMatrixSkipSetEffect`, `rzDevice25Legacy`, `rzDevice25LegacyNoBrightness` (laptop `rzDevice25LedMatrixBlade` also, excluded) | observed (`LedMatrix`, `Legacy`) / inferred (`SkipSetEffect`, `NoBrightness` variants: name only) |
| **per-LED** | 03/02 LED effect + 03/01 LED RGB per LED ID | `rzDevice25LegacyHardwareEffect` | observed; its only registry entry is flagged non-Chroma, so OpenSynapse ignores this family |

Any other class (Noa, Nommo, Glitter, Linker, Esther, Ex, Sage, Oled keyboards with
separate panels, audio/IoT classes, protocol 3.0/4.0) is **unknown** for lighting and gets no
lighting writes.

## Extended family (class 0x0F)

### 0F/02 Set effect, 0F/82 Get effect
Args `[storage, zone, effect, flags, rate, n, n × (R, G, B)]`, data_size `6 + 3·n`
(observed: Synapse sets byte 5 to `6 + colour bytes`). The reply of 0F/82 has the same
layout (observed: parser reads the same fields).

| Byte | Meaning | Tag |
|---|---|---|
| 0 | Synapse calls it "profile". Frames and custom-frame effects always send 0; the background manager's macro-LED helpers send 1. Read here as storage: 0 = volatile (not saved), 1 = saved on the device. OpenRazer names the same values "no store" / "var store". | observed (values) / inferred (meaning) |
| 1 | zone (region ID, table below) | observed |
| 2 | effect ID: 0 off, 1 static, 2 breathing, 3 spectrum, 4 wave, 5 reactive, 6 ripple, 7 starlight, 8 custom frame, 9 fire, 10 audio meter, 11 immersive, 12 wheel | observed |
| 3 | flags (meaning per effect, below) | observed (field) |
| 4 | rate / speed | observed (field) |
| 5 | number of colours `n` | observed |

Per-effect values OpenSynapse sends (Synapse's lighting engine passes flags/rate through
from its UI, whose values are not in the RE material, so these come from OpenRazer):

| Effect | flags | rate | n | Tag |
|---|---|---|---|---|
| off | 0 | 0 | 0 | inferred |
| static | 0 | 0 | 1 | observed (Synapse calls static with one colour, flags 0) / inferred |
| breathing, one colour | 1 | 0 | 1 | inferred (OpenRazer) |
| breathing, two colours | 2 | 0 | 2 | inferred (OpenRazer) |
| spectrum cycling | 0 | 0 | 0 | inferred (OpenRazer) |
| wave | direction 1 or 2 | 0x28 | 0 | inferred (OpenRazer; which direction is "left" is unknown) |

Example (static green on the logo, saved): `0F/02`, data_size 9, args
`01 04 01 00 00 01 00 FF 00`.

### 0F/04 Set brightness, 0F/84 Get brightness
`[storage, zone, value 0–255]`, size 3 (observed). Synapse's UI works in percent:
set `value = floor(percent / 100 · 255)` for this family, read back
`percent = ceil(value / 255 · 100)` (observed). Synapse never reads zone 0: it replaces 0
with the first zone of 0F/80 before a GET (observed), so reading "all" is not known to work.

### 0F/80 Get zone list
Reply: 5 bytes per zone `[zone, updateRate, dataType, arrayX, arrayY]` (observed).
OpenSynapse does not need it yet: zones come from the registry.

## Standard family (class 0x03)

### 03/0A Set matrix effect
No profile/storage byte and no zone byte: the effect applies to the whole device
(observed). There is **no GET** (no 03/8A in the command table, observed), so OpenSynapse
cannot read the old effect back. Byte 0 is a separate effect enum:

| Effect | Args | data_size | Tag |
|---|---|---|---|
| off | `[0]` | 1 | observed |
| wave | `[1, direction]` | 2 | observed (direction values: inferred 1/2, OpenRazer) |
| reactive | `[2, speed, R, G, B]` | 5 | observed (not offered) |
| breathing, one colour | `[3, 1, R, G, B]` | 5 | observed |
| breathing, two colours | `[3, 2, R1, G1, B1, R2, G2, B2]` | 8 | observed |
| breathing, random | `[3, 3]` | 2 | observed (not offered) |
| spectrum cycling | `[4]` | 1 | observed |
| custom frame | `[5, flags]` | 2 | observed (not offered) |
| static | `[6, R, G, B]` | 4 | observed |

Synapse maps its common effect IDs onto this enum: static → 6, breathing → 3, spectrum → 4,
wave → 1, reactive → 2, off → 0 (observed).

Example (static green): `03/0A`, data_size 4, args `06 00 FF 00`.

### 03/03 Set LED brightness, 03/83 Get, 03/89 LED ID list
`[ledClass, ledId, value]`, size 3 (observed). Synapse passes its "profile" argument as
`ledClass` (1 from the background manager, observed), percent →
`value = ceil(percent / 100 · 255)` and back `percent = floor(value / 255 · 100)` (observed;
rounding is the mirror image of the 0x0F pair, each pair is exact for 0–100 %). For LED ID 0
Synapse first asks 03/89 (reply `[count, ids…]`, observed) and uses the first ID, falling
back to 1 (observed). OpenSynapse sends ledClass 1 (inferred: OpenRazer "var store") and
the zone ID as `ledId`; with no zone known it uses 5 (backlight), the usual keyboard LED
(inferred, OpenRazer).

## Zones (region / LED IDs)

One numbering for 0F zones, 03 LED IDs and the background manager's LED constants (observed:
the same table appears three times).

| ID | Zone | ID | Zone |
|---|---|---|---|
| 0 | all / not applicable | 10 | underglow |
| 1 | scroll wheel | 11 | side stripe |
| 2 | DPI LED | 15 | docked |
| 3 | battery LED | 16 | right side |
| 4 | logo | 17 | left side |
| 5 | backlight (keyboard) | 18 | alternate logo |
| 6–9 | actions-minute, macro-recording, game-mode, wireless LEDs | 19–36 | power, suspend, fan, mouse mat, mouse, volume, mute, port stripes 1–6, charging indicators, iOS controller array, knob |

Per device, `data/devices` now records `zones = [...]`: the region IDs of the device
manifest's `ledConfig` (observed for 99 products). Manifest region IDs above 255
(0x40000000, 0x8000000x: virtual/strip regions of the lighting engine) are left out; they
are not command zone IDs (inferred). Products without a manifest region list have no
`zones` field.

## Open questions
- Exact meaning of byte 0 in 0F/02 (storage vs onboard profile slot) — hardware test.
- Wave direction values and speed range for 0x0F — capture of Synapse with a wave effect.
- Whether 0F/82 accepts zone 0 ("all") — hardware test.
- Whether 03/0A effects survive power-off (no storage byte) — hardware test.
