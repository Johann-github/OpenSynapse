# Device database schema

Files: `mice.toml`, `keyboards.toml` (keyboards + keypads), `headsets.toml` (headsets + speakers),
`other.toml` (mousepads, docks, controllers, everything else). Each file is a list of
`[[device]]` tables. Build notes and provenance: `vault/Devices/Device Registry.md`.

Rule: only factual values. A field that is not known is **omitted** (never guessed). The
`evidence` field states where the entry as a whole comes from.

## `[[device]]` fields

| Field | Type | Req. | Meaning |
|---|---|---|---|
| `name` | string | yes | Marketing name as Synapse (or OpenRazer) spells it |
| `vid` | int (hex) | yes | USB vendor id, always `0x1532` so far |
| `pid` | int (hex) | yes | Primary USB product id. For Synapse devices: equals `synapse_product_id` |
| `synapse_product_id` | int | no | Synapse product id (decimal). Equals the USB PID in decimal |
| `kind` | enum | yes | `mouse` `keyboard` `headset` `mousepad` `keypad` `dock` `controller` `speaker` `other` |
| `protocol` | enum | yes | Protocol family spoken on the primary PID: `2.5` `3.0` `4.0` `unknown` |
| `connection` | array | no | Transports the product supports: `usb`, `dongle`, `bluetooth` |
| `dongle_pid` | int or array | no | PID(s) of the wireless receiver, if different from `pid` |
| `bluetooth_pid` | int | no | PID the device reports over Bluetooth |
| `dual_link` | bool | no | Synapse "dual link": shares one HyperSpeed dongle with a second device |
| `laptop` | bool | no | The entry is the built-in keyboard/lighting controller of a Razer laptop |
| `transaction_id` | int (hex) | no | Fixed transaction id OpenRazer uses for this PID (see note) |
| `dpi_min` / `dpi_max` / `dpi_step` | int | no | Sensor range and step as configured in Synapse (mice only) |
| `polling_rates` | int array | no | Polling rates (Hz) offered by the Synapse UI for this product |
| `battery` | bool | no | Present (`true`) only when Synapse reads battery state |
| `lighting` | inline table | no | See below |
| `synapse_class` | string | no | Name of the Synapse device-driver class used for the primary PID |
| `synapse4` | bool | no | `false` = only known from Synapse's legacy (Synapse 2) list, not supported by Synapse 4 |
| `evidence` | enum | yes | `synapse-manifest` `synapse-js` `openrazer` `inferred` |

### `lighting`
- `{ chroma = false }` — Synapse flags the product as non-Chroma (no user lighting).
- `{ matrix = [rows, cols], leds = N, regions = R }` — size of the LED matrix Synapse's
  lighting engine addresses, total LED count over all regions, number of regions. Zone
  names are not in the source data, so there is no `zones` field yet.

### `[[device.variants]]`
Per alternate PID (receiver or Bluetooth) of the same product:

| Field | Type | Meaning |
|---|---|---|
| `pid` | int (hex) | Alternate USB/BLE product id |
| `connection` | string | `dongle` or `bluetooth` |
| `protocol` | enum | Protocol family on that PID, if known |
| `synapse_class` | string | Synapse driver class on that PID, if known |

## Evidence values
- `synapse-manifest` — Synapse product/lighting manifest JSON (per-product metadata files or
  dashboard device lists).
- `synapse-js` — Synapse per-product or background JavaScript only (no manifest for this PID).
- `openrazer` — OpenRazer kernel driver headers/sources only (cross-reference).
- `inferred` — derived from naming/context; treat as a hint.

## Notes
- `protocol` mapping from `synapse_class`: `rzDevice25*` → `2.5`, `rzDevice30*` → `3.0`,
  `rzDevice40*` → `4.0`. Audio/camera/IoT classes (`Audio*`, `rzCam*`, `rzDeviceIoT`, …) use
  vendor-specific transports → `unknown`.
- `transaction_id`: Synapse 4 itself uses a rolling counter in the 2.5 transaction byte
  (see `vault/Protocol/Report Format 2.5.md`); the per-device fixed value is OpenRazer's.
- PIDs are unique across all files (primary `pid`s); alternate PIDs appear only in `variants`.
