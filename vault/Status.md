---
tags: [moc, status]
updated: 2026-11-10
---
# Project status & handoff

> Read this first when resuming. Then [[Home]].

## Where we are
| Phase | State |
|---|---|
| 0 Setup (tools, repo, vault, workspace) | ✅ done |
| 1 Reverse engineering (protocol, transport, device DB) | ✅ done (open questions below) |
| 2 Implementation (protocol, registry, hid, CLI) | ✅ done 2026-11-09 (budget miss below) |
| 3 Hardware test on Viper V3 HyperSpeed + USB capture | 🔄 **in progress** (reads/writes ✅, first capture ✅) |
| 4 Agent (background service) + GUI | ⏳ later |

## ▶ Resume here (next session)
State at end of 2026-11-10: phase 3 done except "off"/"DPI down" captures; phase 4 started.
- CLI: `buttons get|set|list` (mouse-button, double-click, key, dpi-up; normal + HyperShift),
  `dpi-stages get|set --active`, `profile export|apply` (TOML, restore snapshot). All hardware-tested.
- GUI `opensynapse-desktop` (egui, green accent, 4.4 MB): read panel + guarded DPI/polling/sleep writes. User-checked.
- Viper has separate HyperShift slots; reply byte 2 of 02/8C is unknown (not a mode echo).
- Mouse state: defaults, DPI 800 (stage 2), 1000 Hz, sleep 300 s.
→ [[RE-Log/2026-11-10 Onboard button test]], [[RE-Log/2026-11-10 Button capture key and DPI]], [[Decisions/ADR-0004 GUI toolkit egui]]

- Agent slice 1 done (2026-11-10): `opensynapse-agent` + `opensynapse-ipc` (named pipe, own codec) +
  `opensynapse-device` + CLI `agent ping|devices|snapshot|watch|stop`. Hardware-tested: 9.2 MB RAM, idle CPU 0,
  hotplug events work. → [[RE-Log/2026-11-10 Agent slice 1 test]], [[Architecture/Agent]]

Next tasks, in order:
1. ✅ Events found: MI_01 COL06, report 5 (DPI changed observed); agent listens by default.
   Open: battery/charging events (plug the cable in during `hid listen`). → [[RE-Log/2026-11-10 Hardware events capture]]
2. ✅ ApplyProfile in the agent (`opensynapse agent apply`), shared engine in `opensynapse-device`; hardware-tested.
3. ✅ GUI over IPC (agent if running, else direct HID; live events). Hardware check by the user pending.
4. Autostart (HKCU Run, `windows_subsystem`), refuse elevated start; then per-app profiles.
5. Captures for "off" and "DPI down"; power-cycle test for button assignments.
6. Mouse asleep → friendlier error ("move the mouse to wake it").

Before every commit: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.

## Key facts (details in linked notes)
- Viper V3 HyperSpeed = PID 0x00B8, Synapse product id 184, **protocol 2.5**, class `rzDevice25DualLinkMouse`, interface 0, 25 ms timing, no RGB, 100–30000 DPI step 50, 125/500/1000 Hz. → [[Devices/Viper V3 HyperSpeed]]
- Report: 90 bytes (+ report ID 0 = 91 on Windows), XOR CRC bytes 2..87, rolling transaction ID 0..0x1E. → [[Protocol/Report Format 2.5]]
- Commands (DPI 04/05, stages 04/06, polling 00/05 + 00/40, battery 07/80, charging 07/84, sleep 07/03, buttons 02/0C, profiles 05/xx). → [[Protocol/Commands 2.5]]
- Windows: native SetupAPI/HidD backend (ADR-0003; hidapi only off Windows); open MI_00 usage page 0x0001 / usage 0x0002, feature reports only, no read/write. → [[Protocol/Windows Transport]]
- Protocol 3.0 = BLE, 4.0 = 64-byte report ID 2 frame reusing 2.5 commands. → [[Protocol/Protocol 3.0 and 4.0]]
- Events: input report ID 5, byte 1 = event id. → [[Protocol/Hardware Events]]
- Device DB: 448 products in `data/devices/*.toml`, schema `data/devices/SCHEMA.md`. → [[Devices/Device Registry]]

## Phase 2 result (2026-11-09)
- `opensynapse-protocol`: frame/CRC/status/transaction counter, command table, typed
  device/DPI/stages/polling (both variants)/battery/charging/sleep/low-battery/profiles/
  buttons (raw)/lighting 0F + 03, event decoder. 36 unit tests at the end of phase 2, 38 today.
- `opensynapse-device-registry`: embeds `data/devices/*.toml`, parses on first use (~12 ms
  release), lookup by primary → variant → dongle PID, `dpi_valid`. 5 tests.
- `opensynapse-hid`: enumerate, control-collection match (iface 0, UP 0x0001/U 0x0002),
  `Session::execute` with Synapse's SET/GET/busy/resend rules (25 ms, 10 in / 20 out),
  `RazerAppEngine.exe` check (now via ToolHelp snapshot, see ADR-0003). 8 mock tests.
- `opensynapse-cli`: `list`, `info`, `battery`, `dpi get|set`, `dpi-stages`,
  `polling get|set` (auto: period 00/05, falls back to hyper 00/40 on "not supported"),
  `sleep get|set`. Writes = dry run unless `--apply`; read old → write → read back →
  print restore command. Volatile DPI storage by default (`--persistent` opt-in).
- Live check: `opensynapse list` finds `00B8 Razer Viper V3 HyperSpeed (protocol 2.5,
  control: yes)`.
- Hardware reads succeeded: firmware, serial, device mode, battery, charging, DPI,
  DPI stages, polling (00/85 works), sleep (300).
- Hardware writes succeeded with read-back + restore: DPI storage 0 and 1, polling
  00/05, sleep 07/03. → [[RE-Log/2026-11-09 Hardware read test]]
- USB capture of a Synapse button remap decoded: 02/0C uses data_size 10; mouse button =
  function 1 `[buttonId]`, double-click = function 11 `[01]`. → [[RE-Log/2026-11-09 Button remap capture]]

### Budgets ([[Decisions/ADR-0002 Lean by design]])
| Metric | Target | Measured | |
|---|---|---|---|
| CLI binary (release) | < 5 MB | 1.16 MB | ✅ |
| `opensynapse list` | < 200 ms | 75–84 ms | ✅ |
| GUI binary (release) | < 10 MB (own target) | 4.4 MB | ✅ |
| Agent RAM idle | < 15 MB | 9.2 MB WS / 2.0 MB private | ✅ |
| Agent CPU idle | ~0 % | 0 s over 169 s | ✅ |
| device command (`dpi get`) | — | 133–162 ms | |

The first build missed the `list` budget (245–285 ms) because hidapi scans every HID
device. Fixed with a native Windows backend → [[Decisions/ADR-0003 Own Windows HID backend]].

## Phase 3 log (2026-11-09)
- ✅ All read commands on the real mouse (`-v` logs every report).
- ✅ Writes with read-back + restore: DPI (volatile/persistent), polling, sleep.
- ✅ Power-cycle test: DPI storage 0 = volatile, 1 = persistent.
- ✅ USB capture of a Synapse button remap decoded (functions 1 and 11).
- ✅ 2026-11-10: CLI `buttons get|set`; onboard double-click works without Synapse (device mode Normal).
- ✅ 2026-11-10: key (function 2, `[mods, HID usage]`) and DPI up (function 6, `[01]`) decoded; key A works onboard.
- ⏭️ Continue with "Resume here" above.

## Open questions (carry over)
- ~~Storage byte meaning~~ resolved (observed, power-cycle test): 0 = volatile, 1 = persistent; a storage-1 write also sets the active value.
- Viper: polling 00/05 works on the dongle (observed); 00/40 untested. Sleep unit: seconds (inferred, 300 read).
- Button-assignment data bytes: functions 1 and 11 known; key, DPI, macro etc. still unknown.
  ~~Does the onboard mapping act with Synapse closed?~~ yes (observed 2026-11-10).
- Wireshark: the Viper dongle hangs on USBPcap3 (device 3.9). USBPcapCMD.exe had to be copied
  into `%APPDATA%\Wireshark\extcap`.
- Event collection usage page/report on the Viper.
- 81 products without manifest → protocol unknown.

## Environment (dev machine)
- Rust: `~/.cargo/bin` (add to PATH in Git Bash: `export PATH="$HOME/.cargo/bin:$PATH"`).
- REA: `rea` CLI + MCP registered for Claude Code (needs restart to load tools). Node 24.18 (< REA's 24.22, works with warning).
- Ghidra 12.1.4: `~/tools/ghidra_12.1.4_PUBLIC`, env `GHIDRA_INSTALL_DIR` set (user). JDK 21 via winget.
- Wireshark + USBPcap installed. Viper dongle on **USBPcap3**; `USBPcapCMD.exe` copied to
  `%APPDATA%\Wireshark\extcap` (otherwise no USBPcap interfaces). tshark: `C:\Program Files\Wireshark\tshark.exe`.
- RE material (never commit): `~/re-work/`
  - `synapse4/app` extracted app.asar · `synapse4/swcache` service-worker cache dump · `synapse4/bm/assets` background-manager JS · `synapse4/pretty` prettified · `synapse4/products`, `synapse4/fetched` manifests · `native/` native analysis
  - `tools/dump_simple_cache.py` cache dumper · `tools/agent-scripts/` (device DB `extract.py`/`build.py`, command-table dumps `cmds.txt`, `tbl*.md`)

## Known tooling quirk
- Subagents may be blocked from writing `.md` files by the harness ("return findings as text"). Then the main session writes the notes from the agent's report. Code files (`.rs`, `.toml`) are not affected.
