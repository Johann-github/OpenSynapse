# 2026-11-09 Windows transport

**Question:** Does Synapse 4 reach Razer devices on Windows via plain HID, or does it need Razer's filter driver or a service? Which collection, report ID and report size does it use?
**Evidence (source / tool):** PE import/string extraction (small Python scripts under `re-work/native/`; nothing pasted here), extracted shell JS (`app/electron`), background-manager JS (`bm/assets`), lighting-engine `DeviceManifest_184`, driver INFs, read-only PnP enumeration, and a read-only HidP_GetCaps probe (zero-access handles; no reports sent). Result note: [[Protocol/Windows Transport]].

## Method
1. `node-rz-hid/build/Release/HID.node`: dump PE import tables (static + delay) and ASCII/UTF-16 strings.
2. `node-rz-hid` package in `app/node_modules`: `package.json` (v0.0.31), bundled `hidapi/VERSION` (0.10.1). Only build files ship; there is no `hid.c`.
3. JS call graph: `UsbRzDeviceAction.js` actions `hid.getDevices`, `hid.openDevice`, `hid.sendFeatureReport(Mutex)`, `hid.getFeatureReport`, `hid.sendFeatureReportInBatch`. In background-manager: `connectHidDevice` in `index-*.js`, class `rzDevice25` (`reportLength`, `reportId`, `_createDataSend`, `_calculateChecksum`), and `rzDevice25DualLinkMouse.connectDevice` (byte 0 = `claimInterface` when the Windows UA check `ye()` is true).
4. Device config: `DeviceManifest_184_0.json` → `rzDeviceConfig.claimInterface = 0`, `rzDeviceName = rzDevice25DualLinkMouse`, retry/sleep values; `products/184/mw/manifest` → driver package `PID00b8Drv_v23.0.1.0`.
5. Drivers: INFs in `PID00b8Drv\Drivers\x64\{.,KB,Lower,MPos,VCon}` and `RzComDriver\Drivers\x64`. Strings and imports of `RzDev_00b8.sys` (KMDF, v1.0.0.74) and `RzCommon.sys` (KMDF, v1.0.0.76).
6. Who uses the control device: binary scan of all Razer exe/dll/node/sys for `RZCONTROL`/`RzCommon`/`Virtual_Kbd` (ASCII + UTF-16). The only hits are the drivers themselves. Then imports/strings of `CommonDLL\mapping_engine.dll`.
7. Live system: `Get-PnpDevice` + `Get-PnpDeviceProperty` (Service, Upper/LowerFilters, HardwareIds, Parent, INF), then per HID TLC: CreateFile RW vs zero-access, HidD_GetPreparsedData + HidP_GetCaps.

## Findings
- (observed) `HID.node` = hidapi 0.10.1 + node-hid-style N-API binding. `hid.dll` is loaded dynamically (strings `hid.dll`, `HidD_SetFeature`, `HidD_GetFeature`, `HidP_GetCaps`, …). Static imports are only KERNEL32/SETUPAPI/WINMM/ole32. There are no Razer IOCTLs and no RZCONTROL path.
- (observed) Razer extensions: `sendFeatureReportInBatch`, `sendFeatureReportSync`, `getFeatureReportSync`, `readTimeoutAsync`, `setTimerResolution`, plus enumeration fields `deviceContainerId` and `usbInstanceId`.
- (observed) The open path is chosen by `vendorId` + `productId` + `deviceContainerId` + `interface === claimInterface`. Each candidate path is tried with a probe command and the first success is kept. Usage page/usage are only used by `modules/hidHardwareEvents` (input listener, keyed by manifest `hidConfig.usagePage/usageId`).
- (observed) Report: 91 bytes, byte 0 = 0x00. `getFeatureReport(reportId, 91)` result is sliced from byte 1. A send counts as successful when 91 bytes are written.
- (observed) Live caps: `HID\VID_1532&PID_00B8&MI_00` → UP 0x0001 / U 0x0002, In 9, Out 0, **Feature 91**. All other TLCs have Feature 0. MI_01 Col05–08 are UP 0x0001 / U 0x0000 input-only (16 bytes). These are likely the hardware-event / Razer-key input collections (inferred).
- (observed) Access: mouse/keyboard TLCs (MI_00, MI_01 Col01/Col02, MI_02) → RW open = ERROR_ACCESS_DENIED, zero-access open = OK. Consumer/system/vendor TLCs open RW.
- (observed) PnP topology: RzDev_00b8 is a LowerFilter under HidUsb on USB MI_00/MI_01 and an UpperFilter on the mouse/keyboard TLCs. `RZCONTROL\…MI_00` is a child of the HID MI_00 TLC, service RzCommon. `RZVIRTUAL\…MI_00&Col03` is a virtual HID (mshidkmdf) with consumer + system-control TLCs. Its HID children also carry `MI_00` in their path, so hidapi will report interface 0 for them.
- (observed) INFs give the filtered keyboard/mouse TLCs the SDDL `D:P(A;;GA;;;SY)(A;;GRGWGX;;;BA)(A;;GRGWGX;;;WD)(A;;GRGWGX;;;RC)`. Exclusivity still comes from the OS input stack, not from this ACL (inferred).
- (observed) `mapping_engine.dll` (Chromium-base style, sources `mapping_engine\device\driver\win\driver_impl_win.cc`, `endpoint_impl_win.cc`) has `ConnectToDriver`, `ProbeFilterDriverEndpoint`, `ProbeHidEndpoints`, `Enable{Mouse,Keyboard,RazerKey}InputRedirect`, `SendInputToOS` and fallback `addUsbDeviceWithoutFilterDriver`. It imports SetupDi* (delay), DeviceIoControl, IOCP, RawInput, SetWindowsHookExW and SendInput. JS side: `RzMappingEngine.addUsbDevice(device, skipFilterDriver)`.
- (observed) Protocol-2.5 command descriptors for onboard mapping exist: Single Button Assignment, Button Assignment List, Button Mapping Mode, Macro Memory, Profiles.
- (inferred) Plain hidapi (Rust `hidapi` crate, same zero-access fallback upstream) can send and receive configuration with no Razer software installed.

## Open questions
- Verify on a clean machine (no PID00b8Drv/RzCommon) that the MI_00 TLC still exposes Feature 91 and that SetFeature works through the zero-access handle.
- Does RzDev_00b8 (upper filter on the MI_00 TLC) touch IOCTL_HID_SET/GET_FEATURE? Not needed for us, but it explains any behaviour difference with and without the driver. That would need a Ghidra pass over its EvtIoInternalDeviceControl (`CRzDevice::IoInternalDeviceControl`).
- For a Viper V3 button remap in Synapse: onboard (Set Single Button Assignment) or host engine? Do a USBPcap capture while changing one binding.
- What `Set Button Mapping Mode` switches (HID-native vs Razer-key report on MI_01 vendor TLCs)?
- Device-interface GUID that RzCommon registers and the IOCTL set (only relevant if we ever want to cooperate with the Razer driver; currently a non-goal).

## Next
- [[Protocol/Report Format 2.5]]: byte layout of the 90-byte payload (transaction ID, size, class/id, checksum at 88).
- [[Protocol/Commands 2.5]]: onboard mapping commands.
- [[Protocol/Hardware Events]]: MI_01 vendor TLCs and `hidHardwareEvents`.
- Update [[Devices/Viper V3 HyperSpeed]] with the TLC table (MI_00 = mouse + Feature 91, no separate control collection).
