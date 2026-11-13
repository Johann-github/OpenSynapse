---
tags: [moc, status]
updated: 2026-11-12
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
| 4 Agent (background service) + GUI | ✅ done 2026-11-12: agent, autostart, per-app profiles + GUI editor (incl. buttons, volatile), install |
| 5 First release | ✅ **v0.1.0 Windows preview published 2026-11-13** (pre-release) |

## ▶ Resume here (handoff 2026-11-13 → next session)
**v0.1.0 is out:** https://github.com/Johann-github/OpenSynapse/releases/tag/v0.1.0 (pre-release,
zip + SHA-256, built by `.github/workflows/release.yml` from tag `v0.1.0` = `f433c8c`).
Release flow: notes in `docs/release-notes/<tag>.md` → push tag `v*` → workflow makes a **draft**
→ download + check → user clicks Publish. (Tag v0.1.0's workflow wrote the .sha256 with CRLF; fixed
by hand in the release and in the workflow, `2bbc532`.)
Next candidates (ask the user):
1. Live check: GUI battery % updates from the event (leave GUI open a few hours).
2. Linux/macOS hardware test (port branch `linux-macos-port`, stages 1–6 done, CI green), then
   merge it into `main` and switch `main` CI back to all three OSes.
3. Battery event granularity (longer capture), low-battery warning.

### 2026-11-13
- ✅ Protocol 4.0 framing (report ID 2, 64 bytes) in `opensynapse-protocol` (`report40`,
  `framing`) + `opensynapse-hid` (framing from the registry); `Protocol::is_supported()` is
  the one gate (CLI, agent, GUI). **Inferred, no hardware; no registry device is 4.0 yet.**
  3.0 (BLE GATT) planned only → [[Decisions/ADR-0015 Protocol 4.0 and 3.0 support]],
  [[RE-Log/2026-11-13 Protocol 4.0 frame and device search]].
- ✅ Lighting / RGB for protocol 2.5 (**untested on hardware**, no RGB device here) →
  [[Protocol/Lighting 2.5]], [[Decisions/ADR-0014 Lighting]]: extended (0x0F) vs standard (03/0A)
  family from the Synapse driver class, manifest zone IDs in `data/devices` (99 products),
  `opensynapse lighting get|set` (off/static/breathing/spectrum/wave, brightness, zones), GUI
  Lighting section (extended only, accent green default), IPC v4 `WireSetting::Lighting`.
  Profiles `[lighting]` parked in [[Backlog]].
- ✅ Release prep: user README, untested-device write warning (registry `tested = true` only for the
  Viper; CLI stderr line, GUI label, `info` shows "Tested with OpenSynapse"), CI on `main`
  (Windows only until the port merge), release workflow, release notes.
- ✅ Battery-level event observed: one `05 31 3E 00 00 01` (62/255 = 24 %) after ~2.5 h of
  `hid listen` (15:52–~18:20, ended by host memory pressure) → [[RE-Log/2026-11-13 Battery event capture]].
- ✅ App icon drawn in code (`opensynapse-icon`), green accent shared with the GUI
  (`opensynapse_icon::ACCENT`; user: accent always green). Linux/macOS port: branch
  `linux-macos-port`, stages 1–6 done, CI green (see that branch's Status).
- ✅ Start-menu shortcut now has a LinkTargetIDList → [[Decisions/ADR-0009 Install location]].
  ✅ User-checked 2026-11-13 after rebuild + install: Properties shows the target.
  Installed exes built 2026-11-13 from `a2b2bab`.

Mouse state at end of day 2026-11-12: DPI 800 (stages 400/800, active 2, volatile + persistent),
1000 Hz, sleep 300 s, button 4 = mouse button 4 in slot 0 and slot 1, wake hint off.
`app-profiles.toml` has **1 rule from the user test: `notepad.exe` → `profiles/notepad.toml`
(button 4 = DPI down)**; remove it in the GUI if unwanted. Agent installed and running from
`%LOCALAPPDATA%\Programs\OpenSynapse` (autostart points there), all 3 exes current (built
2026-11-12 from `be85d93`). Synapse autostart is off.

Build + update: stop nothing (the installed agent runs from its own folder), then
`cargo build --release --workspace` and `target\release\opensynapse.exe install`.
A plain `cargo build --release` builds only the CLI (`default-members`); `install` now prints a
note when the agent/GUI exe is older than the CLI.

## Done 2026-11-12
1. ✅ **Install (ADR-0009)** done 2026-11-11: `opensynapse install|uninstall`, location
   `%LOCALAPPDATA%\Programs\OpenSynapse`, agent/autostart/GUI prefer the installed agent. Tested
   end-to-end with `--dir` + `--no-autostart`. ✅ User ran the real install (2026-11-11): 3 files
   copied, Run key moved from `target\release` to the installed agent, installed agent running
   (ping OK). After a rebuild (`cargo build --release --workspace`; plain `--release` builds only
   the CLI), run `target\release\opensynapse.exe install` again to update.
   → [[Decisions/ADR-0009 Install location]]
   Later 2026-11-11, installed by the user (update over the running installed agent: stopped,
   3 files "Updated", Start-menu shortcut written, autostart already correct, agent restarted):
   Start-menu shortcut for the GUI (own `.lnk` encoder), piped `install`/`agent start` no longer
   hang, flash-wear warning in the editor. ✅ User: "OpenSynapse" in the Start menu starts the GUI.
2. ✅ **Buttons in the per-app profile editor** (user-checked 2026-11-12 after rebuild + install):
   notepad rule, Side button 4 = DPI down, Save → `profiles/notepad.toml` has `[buttons.4]
   action = "dpi-down"`; in notepad DPI goes down, elsewhere button 4 = browser back. Afterwards
   slot 0 and slot 1 both mouse button 4 (slot 1 never written). DPI reset to 800 / stage 2.
   2026-11-12 user: "cannot reassign a button". Cause: the installed GUI was built 18:49, before the
   editor (20:00); `cargo build --release` had rebuilt only the CLI. Editor code works (headless egui
   click tests). `install` now notes stale binaries (commit `c3073a3`).
3. **Flash wear**: ✅ hardware test 2026-11-12: 02/0C profile byte 0 is **volatile** (acts at once,
   gone after power-off, slot 1 untouched) → [[RE-Log/2026-11-11 Button write storage]].
   ✅ Done 2026-11-12 (unit-tested): per-app switches write buttons to slot 0, editor shows a
   plain "temporary" hint instead of the flash warning (commit `be85d93`). ✅ Hardware-checked with item 2.
   → [[Decisions/ADR-0007 Per-app profiles]], [[Protocol/Commands 2.5]]

Rules for hardware tests: after any DPI-changing test, reset to DPI 800 / stage 2 (volatile + persistent).
Stop the agent before building only if it runs from `target\release` (exe lock); the installed one does not lock the build.

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
