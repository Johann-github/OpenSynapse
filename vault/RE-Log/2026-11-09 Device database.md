# 2026-11-09 Device database

**Question:** Which Razer devices does Synapse 4 know? How does it map product id → USB PID →
protocol? What capabilities can be extracted as facts for `data/devices/`?

**Evidence (source / tool):** cached Synapse files (`re-work/synapse4/products`, `swcache`,
`bm/assets`), public read-only GETs on `apps.razer.com/synapse/...` (raw copies kept in
`re-work/synapse4/fetched/`, never in the repo), and OpenRazer driver headers and sources as a
cross-reference. Extraction scripts lived in a scratch directory. Only reworded facts went into the repo.

## Method
1. Dashboard list `AvailableDevices` gave about 335 Synapse-4 product ids, each with
   optional receiver and BLE ids and a non-Chroma flag.
2. For every product id: fetched `lighting-engine/manifest/DeviceManifest_<id>_0.json` (239
   found, 94 return 404). Also fetched `products/<id>/mw/` (index → main bundle) for UI ranges.
   A few names came from `products/<id>/mw/manifest.json` where the bundle only had a variable.
3. For every receiver/BLE id: fetched its device manifest as well (135 ids, 76 found) to get
   a per-PID driver class.
4. Background-manager JS: parser enum (`hardwareEventParserType` values),
   on-the-fly-macro table (102 PIDs → protocol and class), 2.5 transaction-id behaviour.
5. Further dashboard lists: `Synapse2Devices` (164 legacy names), `XboxDevices`,
   `XboxHeadsets`, `Windows10Devices`, and `DualDongleCompatibleDevices` (42 pairing groups).
6. OpenRazer `razermouse/kbd/kraken/accessory` headers (274 PIDs) gave names and fixed
   transaction ids per PID (majority value used in the driver's per-PID switch cases).
7. Merged by PID. Precedence for names: device manifest, then UI bundle, then OpenRazer.
   Kind: Synapse category, else a name heuristic. Generated the four TOML files.

## Findings
- (observed) Synapse product id equals the USB PID in decimal (VID `0x1532`).
  Example: 184 = `0x00B8`, and its driver package is named `PID00b8`.
- (observed) Device manifests name a driver class (`rzDeviceConfig.rzDeviceName`). It
  determines the protocol family: `rzDevice25*` (2.5), `rzDevice30*` (3.0) and `rzDevice40*` (4.0),
  plus audio/camera/IoT classes. Variants seen: `rzDevice25`, `…LedMatrix`, `…LedMatrixSkipSetEffect`
  (laptops), `…DualLinkMouse`, `…DualLinkKeyboard`, `…ChromaEffect`, `…Legacy*`, `…Oled`,
  `…Linker*`, `…Glitter*`, `…Noa*`, `…Sage`, `…Esther`, and `rzDevice30` / `rzDevice30Oled`.
- (observed) Protocol is chosen per PID. BLE PIDs use `rzDevice30`, and wired and receiver PIDs
  of the same product use `rzDevice25*`. 32 variant PIDs are 3.0 and 42 are 2.5.
- (observed) Only one primary PID uses 3.0 (Head Cushion Chroma, `0x0F36`). No fetched
  manifest references `rzDevice40*`.
- (observed) The parser enum: 1/9/17/18/20/23/24/25/29 are 2.5 parsers, 6/19 are 3.0, 26/27/28 are 4.0,
  and the rest are audio-vendor or fan parsers.
- (observed) The 2.5 base class in Synapse uses a rolling transaction id (0…30). OpenRazer
  instead uses fixed values per PID (`0x1F` 62×, `0x3F` 30×, `0xFF` 113× in our data).
- (observed) UI bundles give DPI min/max/step and polling-rate lists (e.g. DeathAdder V4 Pro
  `0x00BE`… up to 45000 DPI / 8000 Hz; Viper V3 HyperSpeed 30000 DPI / 1000 Hz).
- (observed) The data has errors: the device manifest for `0x00BE` is titled "Viper V3 Pro".
  Its UI bundle and OpenRazer both say DeathAdder V4 Pro, so the registry uses the latter.
- (inferred) Laptops (`category SYSTEM`, "Razer Blade/Book") are recorded as `keyboard` with
  `laptop = true` because the USB device is the keyboard/lighting controller.
- Result: 448 primary entries and 137 variant PIDs. See counts in [[Devices/Device Registry]].
  Viper details: [[Devices/Viper V3 HyperSpeed]].

## Open questions
- 81 Synapse-4 products have no device manifest (new mice such as Viper V4 Pro `0x00E5` and
  Basilisk V4, and all Wolverine/audio/camera products). Is their class defined elsewhere
  (bundle-internal), and which protocol do they speak? (unknown)
- Product ids above `0xB000` (Viper V4 HyperSpeed `0xB005`, DeathAdder V4 HyperSpeed
  `0xB007`, Basilisk V3 Pro 35K Xbox `0xB00A`) do not follow the classic PID numbering.
  Are they real USB PIDs or virtual Synapse ids? (unknown)
- Synapse names some products by codename (e.g. "Razer Krakoff" `0x00DD`, "Razer Clio",
  "Razer Carol T2"). The final product names are unknown.
- Lighting zone names: the manifests only describe matrix positions and regions. A mapping to
  logo/scroll/underglow needs device captures.
- Why does 3.0 show up as a primary only for the Head Cushion? Check its transport.
- Polling-rate lists in the UI may be per connection (wired vs. receiver). We record only the
  default list.

## Next
- Load `data/devices/*.toml` in `opensynapse-device-registry` with a strict schema test.
- Confirm `0x00B8` protocol 2.5 and transaction-id handling on hardware → [[Protocol/Report Format 2.5]].
- Fill the 4.0 gap → [[Protocol/Protocol 3.0 and 4.0]].
