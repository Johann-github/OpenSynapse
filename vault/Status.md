---
tags: [moc, status]
updated: 2026-11-11
---
# Project status & handoff

> Read this first when resuming. Then [[Home]].

## Where we are
| Phase | State |
|---|---|
| 0 Setup (tools, repo, vault, workspace) | ✅ done |
| 1 Reverse engineering (protocol, transport, device DB) | ✅ done (open questions below, rest in [[Backlog]]) |
| 2 Implementation (protocol, registry, hid, CLI) | ✅ done 2026-11-09 |
| 3 Hardware test on Viper V3 HyperSpeed + USB capture | ✅ done 2026-11-11 (all button functions incl. off / DPI down, power cycle) |
| 4 Agent (background service) + GUI | 🔄 **in progress**: agent, autostart, per-app profiles, GUI editor done; install pending |

## ▶ Resume here (handoff 2026-11-11 → next session)
Mouse state at end of day: DPI 800 (stages 400/800, active 2), 1000 Hz, sleep 300 s, button 4 = mouse
button 4, wake hint off, `app-profiles.toml` with 0 rules. Agent installed and running from
`%LOCALAPPDATA%\Programs\OpenSynapse` (autostart points there). Synapse autostart is off.

**Next, in order:**
1. ✅ **Install (ADR-0009)** done 2026-11-11: `opensynapse install|uninstall`, location
   `%LOCALAPPDATA%\Programs\OpenSynapse`, agent/autostart/GUI prefer the installed agent. Tested
   end-to-end with `--dir` + `--no-autostart`. ✅ User ran the real install (2026-11-11): 3 files
   copied, Run key moved from `target\release` to the installed agent, installed agent running
   (ping OK). After a rebuild, run `target\release\opensynapse.exe install` again to update.
   → [[Decisions/ADR-0009 Install location]]
   Later 2026-11-11 (built, not yet installed): Start-menu shortcut for the GUI (own `.lnk`
   encoder, launch tested), piped `install`/`agent start` no longer hang, flash-wear warning in
   the editor. **User step:** run `target\release\opensynapse.exe install` once more, then look
   for "OpenSynapse" in the Start menu.
2. **User check: buttons in the per-app profile editor** (merged, unit-tested): open a rule's
   Settings → Buttons, set Side button 4 = DPI down, Save, check the TOML and the switch. Reset DPI to 800 after.
3. **Flash wear** (open decision for the user): 02/0C button writes are stored onboard, so an app
   profile with buttons writes flash on every switch. Done: skip-unchanged + a warning in the editor
   while a profile sets buttons. Still open: keep buttons in app profiles at all? Look for a volatile
   button write (unknown).
4. Then pick from [[Backlog]].

Rules for hardware tests: after any DPI-changing test, reset to DPI 800 / stage 2 (volatile + persistent).
Stop the agent before `cargo build --release` (exe lock) or build with `--target-dir`.

## Done 2026-11-10 / 11
1. ✅ Events found: MI_01 COL06, report 5 (DPI changed observed); agent listens by default.
   Charging events: n/a (AA battery). Battery-level events → [[Backlog]]. → [[RE-Log/2026-11-10 Hardware events capture]]
2. ✅ ApplyProfile in the agent (`opensynapse agent apply`), shared engine in `opensynapse-device`; hardware-tested.
3. ✅ GUI over IPC (agent if running, else direct HID; live events). User-checked: "Backend: agent", DPI button press updates the GUI live.
4. ✅ Autostart (2026-11-11): `opensynapse agent start` + `agent autostart [on|off]` (HKCU Run), agent is
   `windows_subsystem` with console attach, refuses elevated start (exit 3). New crate `opensynapse-sys`.
   Tested: start 141 ms, no window, 8.4 MB WS. ✅ Re-login check (2026-11-11): Run key set, agent up
   ~55 s after Explorer, 9.3 MB WS, `agent ping` + `agent devices` OK. ✅ Elevated refusal (2026-11-11): admin terminal prints the
   "refusing to run as administrator" message (user-checked). User disabled Synapse autostart.
   → [[Decisions/ADR-0006 Agent autostart]]
4b. ✅ GUI agent controls (2026-11-11): "Start agent at login" checkbox (+ Fix when it points elsewhere),
   "Start agent" button in direct-HID mode; shared `opensynapse_sys::spawn_agent_detached`. User: autostart on via CLI.
4c. ✅ Per-app profiles (2026-11-11): `%APPDATA%\OpenSynapse\app-profiles.toml`, WinEvent foreground hook,
   `agent app-profiles [reload]`, IPC v2. Hardware-tested on the Viper (switch ~112 ms, restore verified).
   → [[Decisions/ADR-0007 Per-app profiles]], [[RE-Log/2026-11-11 Per-app profile hardware test]]
   ✅ Hotplug (2026-11-11): a device plugged in while an app profile is active gets it (restore = its state
   at arrival); asleep mouse → one error, retry on its next event / rescan. Hardware 2026-11-11: mouse took
   the profile at re-arrival, but switching away left 1600 DPI (wireless mouse keeps volatile settings while
   the dongle is out; removal had dropped the restore values). Fixed: removal keeps them, a returning device
   gets them back. GUI now re-reads after `AppProfile` events. ✅ Re-check (user): replug during notepad
   profile → 1600 kept, switch away → 800 (GUI and device).
   ✅ GUI editor (2026-11-11, user-checked: add rule, auto profile path, DPI stages, save + switch): top-bar "Per-app profiles" window lists rules
   (exe → profile file), add/edit/remove/reorder, default profile, settings of the rule's profile
   (PID, DPI stages, polling, sleep; buttons kept), inline errors, atomic save, then `ReloadAppProfiles`
   if the agent runs. Writer `AppProfiles::to_toml` in core (round-trip tests). GUI 5.05 MB.
   🔄 Buttons editable in the rule's profile (2026-11-11): normal + HyperShift rows, "Unchanged" or
   the main panel's actions, left/right-click opt-in; GUI 5.13 MB; unit-tested, **user check pending**
   (open a rule's Settings, set Side button 4 = DPI down, Save, check the file and the switch).
   ✅ GUI "Start agent" click (2026-11-11, user-checked): switches to "Backend: agent".
5. ✅ Captures for "off" (function 0, no data) and "DPI down" (function 6, `[02]`), both hardware-tested
   (2026-11-11). → [[RE-Log/2026-11-11 Button capture off and DPI down]]. ✅ Power-cycle test: 02/0C assignments survive
   switching the mouse off/on (stored onboard).
6. ✅ Mouse asleep hint (2026-11-11, hardware-tested: [[RE-Log/2026-11-11 Wake hint hardware test]]): `HidError::NoResponse` (retries used up,
   last reply status 0x04 Timeout; observed in the slice-1 test), IPC `ErrorKind::NoResponse` (IPC v3),
   `settings.toml` + `opensynapse settings get|set wake-hint on|off`, GUI checkbox "Show hint when the
   mouse is asleep". **Default off.** → [[Decisions/ADR-0008 User settings and wake hint]]
   To check: `opensynapse settings set wake-hint on`, let the mouse sleep (> sleep timer), run
   `opensynapse dpi get` (and `agent snapshot`): expect `Error: … no response from the device …` then `Hint: …`.

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
| GUI binary (release) | < 10 MB (own target) | 5.05 MB (2026-11-11, with app-profile editor) | ✅ |
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
- ~~Button-assignment data bytes~~ known for off (0), mouse button (1), key (2), DPI up/down (6), double-click (11); macro etc. unknown (no use yet).
  ~~Does the onboard mapping act with Synapse closed?~~ yes (observed 2026-11-10).
- Wireshark: the Viper dongle hangs on USBPcap3 (device 3.9). USBPcapCMD.exe had to be copied
  into `%APPDATA%\Wireshark\extcap`.
- ~~Event collection usage page/report on the Viper~~ MI_01 COL06, report 5 (2026-11-10).
- Viper power: one AA battery, no charging (user 2026-11-11) → no charging events.
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
