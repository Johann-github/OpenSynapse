---
vid: 0x1532
pid: 0x00B8
protocol: "2.5"
---
# Razer Viper V3 HyperSpeed

Observed on dev machine (Windows PnP): composite USB device via HyperSpeed dongle,
interfaces MI_00 (mouse + control collections), MI_01 (keyboard/consumer/system
collections), MI_02 (keyboard). Razer filter driver present (`RZCONTROL\VID_1532&PID_00B8&MI_00`).
Synapse product id: `184` (manifest `DeviceManifest_184`).

Protocol, capabilities: see RE notes. Links: [[Protocol/Overview]].

## Capabilities (from Synapse product 184 metadata, 2026-11-09)

Registry entry: `data/devices/mice.toml` (`pid = 0x00B8`). Sources: lighting-engine device
manifest 184, product UI bundle `products/184/mw`, dashboard lists. Background:
[[Devices/Device Registry]], [[RE-Log/2026-11-09 Device database]].

| Property | Value | Confidence |
|---|---|---|
| Category | mouse | observed |
| Connection | HyperSpeed receiver only. The product id *is* the receiver PID (`dongleProductId = 184`, `wiredProductId = 0`, no BLE id) | observed |
| Wired data mode | no separate wired PID known to Synapse. Whether the cable carries data is unknown | unknown |
| Synapse driver class | `rzDevice25DualLinkMouse` → protocol **2.5** | observed (class), inferred (family) |
| Event parser | `hardwareEventParserType = 1` (PROTOCOL25) | observed |
| HID interface claimed | 0 | observed |
| Synapse I/O pacing | retry limit 20 in / 20 out, 25 ms between OUT, IN and OUT→IN | observed |
| Dual link | yes. The receiver can also serve one keyboard. Compatible keyboard receivers: `0x025C` `0x0271` `0x02BA` `0x0290` `0x0296` `0x02D2` `0x02D5` `0x02C9` `0x02DA` | observed |
| HyperPolling | listed as a pairing partner of the HyperPolling Wireless Dongle (`0x00B3`) | observed |
| DPI | 100 – 30000, step 50, separate X/Y supported | observed |
| Default DPI stages | 5 stages: 400 / 800 / 1600 / 3200 / 6400, default 800 | observed |
| Polling rates (Synapse UI, this receiver) | 125, 500, 1000 Hz | observed |
| Power source | one replaceable **AA battery**; no built-in rechargeable cell, so no charging (user, 2026-11-11). Charging status 07/84 reads "not charging" (observed). Charging events cannot occur on this mouse | observed (user + read) |
| Battery | battery-state events supported. The lighting-engine "battery monitoring" flag is false (that is a lighting feature) | observed |
| Low-power default | 30 (unit presumably % battery) | observed (value), inferred (unit) |
| Lighting | none. Flagged non-Chroma; the manifest carries only a 1×1 placeholder matrix | observed |
| Onboard memory | 1 profile slot, hardware profile switch, onboard DPI and polling rate (no onboard macros/brightness) | observed |
| Remappable inputs | left, right, middle, wheel up/down, buttons 4/5, DPI-cycle button (bottom) | observed |
| Hypershift | supported (second mapping layer) | observed |
| Lift-off / landing | asymmetric cut-off, up to 26 levels (product description) | observed |
| Firmware report ids | 4 = Razer key report, 5 = hardware event report | observed |
| Transaction id | OpenRazer uses `0x1F` for this PID. Synapse uses a rolling counter | observed (OpenRazer) |
| Windows driver | Synapse installs a per-PID filter-driver package for `PID00b8` | observed |

## Open questions
- Does the USB cable expose a data interface (and which PID)? Check with a USB capture.
- Polling rates above 1000 Hz with the HyperPolling dongle: which PID and command apply?
- Unit and range of the low-power threshold.
