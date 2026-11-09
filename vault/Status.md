---
tags: [moc, status]
updated: 2026-11-09
---
# Project status & handoff

> Read this first when resuming. Then [[Home]].

## Where we are
| Phase | State |
|---|---|
| 0 Setup (tools, repo, vault, workspace) | ✅ done |
| 1 Reverse engineering (protocol, transport, device DB) | ✅ done (open questions below) |
| 2 Implementation (protocol, registry, hid, CLI) | ⏭️ **next** |
| 3 Hardware test on Viper V3 HyperSpeed + USB capture | ⏳ after phase 2 |
| 4 Agent (background service) + GUI | ⏳ later |

## Key facts (details in linked notes)
- Viper V3 HyperSpeed = PID 0x00B8, Synapse product id 184, **protocol 2.5**, class `rzDevice25DualLinkMouse`, interface 0, 25 ms timing, no RGB, 100–30000 DPI step 50, 125/500/1000 Hz. → [[Devices/Viper V3 HyperSpeed]]
- Report: 90 bytes (+ report ID 0 = 91 on Windows), XOR CRC bytes 2..87, rolling transaction ID 0..0x1E. → [[Protocol/Report Format 2.5]]
- Commands (DPI 04/05, stages 04/06, polling 00/05 + 00/40, battery 07/80, charging 07/84, sleep 07/03, buttons 02/0C, profiles 05/xx). → [[Protocol/Commands 2.5]]
- Windows: plain hidapi works; open MI_00 usage page 0x0001 / usage 0x0002, feature reports only, no read/write. → [[Protocol/Windows Transport]]
- Protocol 3.0 = BLE, 4.0 = 64-byte report ID 2 frame reusing 2.5 commands. → [[Protocol/Protocol 3.0 and 4.0]]
- Events: input report ID 5, byte 1 = event id. → [[Protocol/Hardware Events]]
- Device DB: 448 products in `data/devices/*.toml`, schema `data/devices/SCHEMA.md`. → [[Devices/Device Registry]]

## Phase 2 plan (next session)
Use agents in parallel where independent; follow `AGENTS.md` (clean-room + lean rules).
1. `opensynapse-protocol`: `Report` (90-byte) encode/decode, CRC, status enum, transaction counter, typed commands (device info, DPI, stages, polling both variants, battery, charging, sleep, low-battery, buttons, profiles, lighting 0F + 03), event decoder. Unit tests with hand-built byte vectors.
2. `opensynapse-device-registry`: load `data/devices/*.toml` (embed at compile time with `include_str!`), lookup by VID/PID incl. variants.
3. `opensynapse-hid`: hidapi enumerate/match (usage page/usage/interface), zero-access open, send/get feature report with 25 ms pacing + retry/busy logic from [[Protocol/Report Format 2.5]]; warn if `RazerAppEngine.exe` runs.
4. `opensynapse-cli`: `list`, `info`, `battery`, `dpi get|set`, `dpi-stages`, `polling get|set`, `sleep get|set`. Writes opt-in, read first.
5. Measure budgets from [[Decisions/ADR-0002 Lean by design]] (binary size, startup).

## Phase 3 plan
- Read-only commands first on the real mouse (firmware, serial, battery, DPI get).
- USB capture with Wireshark + USBPcap while changing settings in Synapse, to resolve open questions.

## Open questions (carry over)
- Storage byte meaning in DPI/polling (0 volatile / 1 persistent?).
- Viper: polling command 00/05 vs 00/40; sleep-timer unit.
- Button-assignment data bytes per function type.
- Event collection usage page/report on the Viper.
- 81 products without manifest → protocol unknown.

## Environment (dev machine)
- Rust: `~/.cargo/bin` (add to PATH in Git Bash: `export PATH="$HOME/.cargo/bin:$PATH"`).
- REA: `rea` CLI + MCP registered for Claude Code (needs restart to load tools). Node 24.18 (< REA's 24.22, works with warning).
- Ghidra 12.1.4: `~/tools/ghidra_12.1.4_PUBLIC`, env `GHIDRA_INSTALL_DIR` set (user). JDK 21 via winget.
- Wireshark + USBPcap installed (USBPcap active after reboot).
- RE material (never commit): `~/re-work/`
  - `synapse4/app` extracted app.asar · `synapse4/swcache` service-worker cache dump · `synapse4/bm/assets` background-manager JS · `synapse4/pretty` prettified · `synapse4/products`, `synapse4/fetched` manifests · `native/` native analysis
  - `tools/dump_simple_cache.py` cache dumper · `tools/agent-scripts/` (device DB `extract.py`/`build.py`, command-table dumps `cmds.txt`, `tbl*.md`)

## Known tooling quirk
- Subagents may be blocked from writing `.md` files by the harness ("return findings as text"). Then the main session writes the notes from the agent's report. Code files (`.rs`, `.toml`) are not affected.
