# 2026-11-09 Target survey

**Tools:** REA (rea-agents 6.8), Ghidra 12.1.4, Wireshark + USBPcap.

## Findings
- Synapse 4 = RazerAppEngine 4.0.896 (Electron). See [[Architecture/Synapse 4 Anatomy]].
- `app.asar` (98 MB) contains only shell code; HID via native `node-rz-hid` (`HID.node`).
- Device logic is in the `background-manager` web app (served from `apps.razer.com`,
  cached by Chromium service worker). Extracted with a simple-cache dumper (1629 entries).
- Modules per protocol generation: `rzDevice25*`, `rzDevice30`, `deviceCommandEnum`,
  `usbUsageEnum`, and ~35 `rzHardwareEvents*Parser` modules.
- Dev device: Viper V3 HyperSpeed, VID `0x1532`, PID `0x00B8`, Synapse product id `184`.

## Next
- Protocol 2.5 report format + command table → [[Protocol/Report Format 2.5]]
- Device database extraction → [[Devices/Device Registry]]
- Native transport (`HID.node`) → [[Protocol/Windows Transport]]
