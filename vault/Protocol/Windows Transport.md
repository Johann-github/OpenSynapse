# Windows transport (how Synapse 4 reaches the device)

Up: [[Protocol/Overview]] · Device: [[Devices/Viper V3 HyperSpeed]] · Log: [[RE-Log/2026-11-09 Windows transport]]

Tags: **observed** = seen directly in shipped artifacts or on the live system; **inferred** = follows from evidence + public knowledge (e.g. upstream hidapi source), not yet tested end-to-end; **unknown** = open.

## TL;DR

- Configuration traffic needs **no Razer driver and no Razer service**. Synapse 4 itself uses a stock **hidapi 0.10.1** build (`node-rz-hid`) and sends plain **HID feature reports** (HidD_SetFeature / IOCTL_HID_GET_FEATURE) to the **MI_00 mouse top-level collection** (Usage Page 0x0001, Usage 0x0002). (observed)
- Feature report length is **91 bytes = report ID 0x00 + 90-byte Razer payload**. (observed)
- That collection can't be opened read/write (the OS input stack holds it exclusively), but a **zero-access open works**, and feature reports go through such a handle. hidapi falls back to this on its own. (observed for the open, inferred for SetFeature)
- The Razer kernel drivers (`RzDev_00b8`, `RzCommon`) are there for **host-side input remapping and macros** (the mapping engine). Configuration doesn't go through them.

## Q1 — Win32 APIs and the path Synapse opens

- `HID.node` (package `node-rz-hid` 0.0.31) is a node-hid–style N-API wrapper around **hidapi**. The bundled `hidapi/VERSION` is `0.10.1`. Exported symbols/strings include `hid_open_path`, `hid_send_feature_report`, `hid_get_feature_report`, `hid_write`, `hid_read_timeout` and `hid_enumerate`. (observed)
- Static imports are `KERNEL32` (`CreateFileA/W`, `DeviceIoControl`, `ReadFile`, `WriteFile`, `GetOverlappedResult`, `CancelIo`, `LoadLibraryA`, `GetProcAddress`), `SETUPAPI` (`SetupDiGetClassDevsA`, `SetupDiEnumDeviceInterfaces`, `SetupDiGetDeviceInterfaceDetailA`, `SetupDiGetDevicePropertyW`, …) and `ole32!StringFromGUID2`. **`hid.dll` is not statically imported.** Strings `hid.dll`, `HidD_SetFeature`, `HidD_GetFeature`, `HidD_GetAttributes`, `HidD_GetPreparsedData`, `HidP_GetCaps`, `HidD_GetInputReport`, … show it is loaded at runtime via LoadLibrary/GetProcAddress, which is classic hidapi-windows behaviour. (observed)
- No reference to `RZCONTROL`, `RZVIRTUAL`, `RzCommon` or any Razer device-interface GUID exists in `HID.node`. There is no Razer-specific IOCTL path. `DeviceIoControl` is hidapi's own use (IOCTL_HID_GET_FEATURE / GET_INPUT_REPORT). (observed: absence of strings; inferred: what the IOCTLs are)
- Razer additions on top of node-hid: `sendFeatureReportSync`, `getFeatureReportSync`, `sendFeatureReportInBatch` (several reports in one native call, used for lighting frames), `readTimeoutAsync`, `setTimerResolution` (`timeBeginPeriod`), and the enumeration fields `deviceContainerId` and `usbInstanceId` (derived from the `USB\VID_%04X&PID_%04X\` parent). (observed)
- **Path selection (background-manager JS):** `hid.getDevices` returns the hidapi enumeration. `connectHidDevice` filters by `vendorId`, `productId` and `deviceContainerId`, then picks entries whose **`interface === claimInterface`**. For each candidate it tries `hid.openDevice` with that `path` and sends a probe command (`queryData`). The first one that succeeds wins. Usage page and usage are **not** used to pick the command path. They only matter for the separate `hidHardwareEvents` input-listener path. (observed)
- For PID 0x00B8 (Synapse product 184), `DeviceManifest_184` has `rzDeviceConfig.claimInterface = 0` and `rzDeviceName = rzDevice25DualLinkMouse` (protocol 2.5). The open path is therefore **MI_00**. (observed)

## Q2 — Report size and report ID

- In `rzDevice25`, `reportLength = 91`. Sends report success only if the bytes written equal 91. `hid.getFeatureReport` defaults to length 91 and strips byte 0 (`slice(1)`) before parsing. (observed)
- Byte 0 is the report ID. On Windows the JS puts `claimInterface` there, which is **0**. (On other platforms it uses `reportId`, which defaults to 0.) (observed)
- The checksum is the XOR over payload bytes 2..87 (see [[Protocol/Report Format 2.5]]). (observed)
- Live HidP_GetCaps on `HID\VID_1532&PID_00B8&MI_00` gives **UsagePage 0x0001, Usage 0x0002, InputReportByteLength 9, OutputReportByteLength 0, FeatureReportByteLength 91**. It is the **only** collection of the device with a feature report. (observed, read-only probe, nothing sent)
- Transport is **feature reports in both directions**: SetFeature, then a GetFeature poll for the response. There is no output report (OutLen 0), so `hid_write` isn't an option on MI_00. `write`/`writeAsync` in JS are used only by other device classes (OLED flashing). (observed)

## Q3 — What the Razer drivers do, and whether plain HID works without them

| Node (live PnP) | Driver / filter | Role |
|---|---|---|
| `USB\…&MI_00`, `USB\…&MI_01` | HidUsb, **LowerFilters = RzDev_00b8** | KMDF filter below the HID miniport (INF flags `MouseExDevice`, `DkmKeyDevice`, `DkmMouseDevice`, `MouseEx2Device`, `Virtual_Con`) (observed) |
| `HID\…&MI_00` (mouse TLC), `MI_01&Col01/Col02`, `MI_02` | mouhid/kbdhid, **UpperFilters = RzDev_00b8** | Input filter on the mouse/keyboard collections. INF flag `ControlDevice=1` on MI_00 (observed) |
| `RZCONTROL\VID_1532&PID_00B8&MI_00` | service **RzCommon** ("Razer Control Device") | Child PDO spawned by RzDev_00b8 (format string `RZCONTROL\VID_%04X&PID_%04X&MI_%02X`). RzCommon has `CLIENT_CONTEXT`, `CRzClient::Notify` and a queue ("QueueFull"). It is a user↔kernel channel for input events and injection (observed strings; role inferred) |
| `RZVIRTUAL\…&MI_00&Col03` | RzDev_00b8 + `mshidkmdf` | **Virtual HID** (`RzDevCreateVirtualHidDevice`, `VIRTUALHIDDEVICE_CONTEXT`) exposing consumer-control + system-control collections. The driver also knows `Virtual_Kbd/Mou/Js/Con`. Used to inject remapped output (observed strings; role inferred) |

- **Who talks to the control device:** no Razer user-mode binary contains `RZCONTROL`/`RzCommon` as a string. The consumer is `CommonDLL\mapping_engine.dll` (loaded via `ffi-napi-rz` from `modules/mapping_engine/win`). It has `ConnectToDriver`, `ProbeFilterDriverEndpoint`, `EndpointImpl` and `driver_impl_win.cc`, opens the endpoint by device-interface GUID (SetupDi* + CreateFile + DeviceIoControl + IOCP), and has a fallback mode `addUsbDeviceWithoutFilterDriver` (raw input, `SetWindowsHookExW`, `SendInput`). (observed)
- **Plain HID without the drivers:** the 91-byte feature report belongs to the device's own MI_00 report descriptor. Windows' inbox HidUsb/HidClass exposes it, and Synapse's own config path is plain hidapi, which never touches RZCONTROL. With the stock `msmouse.inf` stack (no RzDev filter) the same collection should exist with the same caps. (inferred, high confidence. To verify on a machine without PID00b8Drv.)
- **Sharing and exclusive access (live probe):** on the MI_00 mouse TLC, `CreateFile(GENERIC_READ|GENERIC_WRITE, FILE_SHARE_READ|WRITE)` fails with **ERROR_ACCESS_DENIED (5)**, and `CreateFile(desiredAccess = 0)` **succeeds**. The same holds for the other keyboard/mouse TLCs (MI_01 Col01/Col02, MI_02). Non-system TLCs (consumer, system control, vendor UP 0x0001/U 0x0000 on MI_01 Col05–08) open read/write. (observed)
  - So a non-Razer app can open the command collection, **but only with zero access**. HidD_SetFeature/HidD_GetFeature are allowed on such handles. hidapi ≥0.10 retries with zero access when the RW open fails ("system devices… feature reports can still be sent"), which is what Synapse relies on too. (inferred from upstream hidapi behaviour; Synapse's success is indirect evidence)
  - Opens are shared (`FILE_SHARE_READ|WRITE`), so **Synapse and OpenSynapse can both hold the device at once**. Nothing at the OS level stops interleaving. Synapse serialises with its own mutex and Web Lock (`hid.sendFeatureReportMutex`). OpenSynapse should refuse to run, or warn, while Synapse/RazerAppEngine is running. (observed for sharing, inferred for the risk)

## Q4 — How button remapping works on Windows

There are two mechanisms:
1. **Onboard / firmware.** Protocol-2.5 commands exist for `Set Single Button Assignment`, `Get Button Assignment List`, `Set Button Mapping Mode`, macro memory (`Set Macro Memory Data`), profiles (`Set Active Profile ID`, `Set Profile …`). These travel over the same MI_00 feature report and persist on the device. (observed command descriptors; per-device use for 0x00B8 is unknown, see [[Protocol/Commands 2.5]])
2. **Host-side mapping engine.** `mapping_engine.dll` + RzDev_00b8 filter + RZCONTROL + RZVIRTUAL. The filter can swallow or redirect physical input (`EnableMouseInputRedirect`, `EnableKeyboardInputRedirect`, `EnableRazerKeyInputRedirect`, `razerKeyReportID`), and output is injected through the virtual HID or `SendInput`. Macros, Hypershift, app-specific mappings and "unsupported mapping" fallbacks run here. Without the filter driver it degrades to a hook/raw-input mode. (observed strings; exact split per feature inferred)
- The manifest for 184 has `isRegisterInputEvents: true` and `hardwareEventParserType: 1`, so Synapse also listens for device hardware events. (observed) Whether a plain remap of a Viper V3 button is written onboard or done in the host engine: **unknown**. Next step is a USB capture of a remap change (see log).

## Recommendation for `opensynapse-hid` (Windows, `hidapi` crate)

1. Enumerate with `HidApi::device_list()`. Match `vendor_id == 0x1532`, `product_id == <pid>`, `interface_number == <claimInterface from device registry>` (0 for 0x00B8), **and** `usage_page == 0x0001 && usage == 0x0002`. Usage matters because the RZVIRTUAL children (`…MI_00&Col03&Col0x`) also parse as interface 0 but carry no feature report.
   - Extra safety: pick the collection whose caps report a 91-byte feature report, i.e. probe with `get_feature_report` returning 91.
2. `open_path(path)`. hidapi falls back to a zero-access handle automatically. **Don't** call `read()`/`write()` on it: there's no output report, and input reads are denied.
3. Send with `send_feature_report(&buf)`, `buf.len() == 91`, `buf[0] = 0x00` (report ID), `buf[1..91]` = Razer 90-byte payload. Receive with `get_feature_report(&mut buf)`, `buf[0] = 0x00`, `buf.len() == 91`, and parse `buf[1..]`. Use Synapse-like pacing: ~25 ms between out/in and up to 20 retries (manifest `sleepTimeBetween*`, `maxRetry*`).
4. No dependency on RzCommon/RzDev drivers, Razer services or admin rights. Detect a running `RazerAppEngine.exe` and warn, because both apps can interleave on the shared handle.
5. Host-side remapping (macros, Hypershift) is **out of scope** for the transport crate. Only onboard assignments via feature reports are reachable without a driver.
