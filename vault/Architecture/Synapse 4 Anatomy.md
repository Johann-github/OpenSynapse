# Synapse 4 Anatomy (RE target map)

Observed on Synapse 4 / RazerAppEngine 4.0.896, Windows 11, 2026-11-09.

| Layer | Location | What it does |
|---|---|---|
| Electron shell | `Program Files\Razer\RazerAppEngine\app-4.0.896\resources\app.asar` | Window/tray host, loads web apps, exposes native modules |
| Native HID bridge | `app.asar.unpacked\node_modules\node-rz-hid\...\HID.node` | Sends/receives HID reports for the JS layer |
| Other native modules | `rz-usb-detect`, `node-ble-rz`, `usb`, `ffi-napi-rz` | USB hotplug, BLE, raw USB, FFI into DLLs |
| Web apps (cached) | `%LOCALAPPDATA%\Razer\RazerAppEngine\User Data\Default\Service Worker\CacheStorage` | Served from `apps.razer.com`, cached by service worker |
| **Protocol logic** | web app `background-manager` → `rzDevice25*`, `rzDevice30`, `deviceCommandEnum`, `rzHardwareEvents*Parser` | Builds HID reports per protocol generation, parses device events |
| Device manifests | `apps.razer.com/synapse/products/<id>/...`, `lighting-engine/manifest/DeviceManifest_<id>` | Per-product capability data |
| Native helpers | `User Data\Apps\Common\` (RzDLLService, lighting_driver, FWUpdate SDKs) | Lighting driver, firmware update (out of scope) |

Key insight: device configuration logic is **JavaScript**, not native code. The native
`HID.node` is a thin transport. See [[Protocol/Overview]].
