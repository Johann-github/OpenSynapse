# OpenSynapse

**A native, local-first alternative to Razer Synapse, written in Rust.**
Configure DPI, DPI stages, polling rate, sleep timer and button assignments of Razer mice,
with per-app profiles. No account, no cloud, no telemetry, no ads. Plain-TOML config.

> **Status: Windows preview (v0.1.0).** Hardware-tested with one device so far, the
> **Razer Viper V3 HyperSpeed**. The registry knows ~450 Razer products; other protocol-2.5
> devices may work, and OpenSynapse warns before writing to one that has not been tested.
> Linux and macOS support is in progress on the `linux-macos-port` branch.

## What it does
| Feature | CLI | GUI | Notes |
|---|---|---|---|
| Device list, firmware, serial, battery | ✅ | ✅ | battery updates live from the device's own event |
| DPI and DPI stages | ✅ | ✅ | volatile by default, `--persistent` stores onboard |
| Polling rate (125 / 500 / 1000 Hz) | ✅ | ✅ | |
| Sleep timer | ✅ | ✅ | |
| Button assignments (mouse button, key, double-click, DPI up/down, off) | ✅ | ✅ | stored onboard, work without OpenSynapse running |
| Per-app profiles (switch settings when an app gets focus) | ✅ | ✅ | needs the background agent |
| Profiles as TOML (export / apply / restore) | ✅ | | |

Not supported (yet): lighting/RGB, macros, firmware updates (OpenSynapse never flashes firmware).

## Install (Windows 10/11, x64)
1. Download `OpenSynapse-<version>-windows-x64.zip` from the
   [Releases](https://github.com/Johann-github/OpenSynapse/releases) page and unzip it.
   The exes are not code-signed yet, so Windows SmartScreen may ask once ("More info" →
   "Run anyway"). The release lists a SHA-256 checksum for the zip.
2. Close Razer Synapse (it talks to the same device and both would interfere).
3. In the unzipped folder, run:
   ```
   opensynapse.exe install
   ```
   This copies the three programs to `%LOCALAPPDATA%\Programs\OpenSynapse`, adds an
   **OpenSynapse** entry to the Start menu, turns on the background agent at login (per-user,
   no admin rights) and starts it.
4. Open **OpenSynapse** from the Start menu.

Uninstall: `opensynapse uninstall` (stops the agent, removes autostart, Start-menu entry and
programs; your settings in `%APPDATA%\OpenSynapse` stay).

## Programs
| File | What it is |
|---|---|
| `opensynapse-desktop.exe` | the GUI (Start menu: **OpenSynapse**) |
| `opensynapse-agent.exe` | small background agent (~9 MB RAM, ~0 % CPU idle): owns the device, per-app profiles, live events |
| `opensynapse.exe` | command line |

## Command line, quick start
```
opensynapse list                       # connected Razer devices
opensynapse info                       # firmware, serial, capabilities
opensynapse dpi get
opensynapse dpi set 1600               # dry run: shows what would be sent
opensynapse dpi set 1600 --apply       # writes, reads back, prints the restore command
opensynapse buttons set 4 dpi-down --apply
opensynapse profile export --out my.toml
opensynapse agent autostart on|off
```
Every write is a dry run unless you add `--apply`; after writing, OpenSynapse reads the value
back and prints how to restore the old one. `opensynapse <command> --help` lists all options.

## Your device isn't tested yet?
Help us mark it as tested:
1. Connect the device (wake a wireless mouse by moving it) and run
   ```
   opensynapse report
   ```
   It only reads (nothing is written to the device, nothing is sent anywhere) and prints a
   Markdown block. Serial numbers are masked.
2. Open a [Device report](https://github.com/Johann-github/OpenSynapse/issues/new?template=device-report.yml)
   issue, paste the output and tick what you tried and what worked.

## Privacy and footprint
No network access at all, no login, no telemetry, no background updater. Config lives in
`%APPDATA%\OpenSynapse` as TOML. Budgets: CLI < 5 MB, agent < 15 MB RAM idle,
`opensynapse list` < 200 ms ([ADR-0002](vault/Decisions/ADR-0002%20Lean%20by%20design.md)).

## For contributors
This repository is also an Obsidian vault: research notes in `vault/` (start at
`vault/Status.md` and `vault/Home.md`), code in `crates/`, device data in `data/devices/`.
Each crate's `README.md` links to the vault notes it implements. Agent/contributor rules:
[AGENTS.md](AGENTS.md).

```sh
cargo build --release --workspace     # CLI, agent and GUI
cargo test --workspace
```

Inspired by [OpenLogi](https://github.com/AprilNEA/OpenLogi) (the same idea for Logitech).

## Legal
OpenSynapse is an independent project, not affiliated with or endorsed by Razer Inc.
"Razer" and "Synapse" are trademarks of Razer Inc. Reverse engineering is done solely for
interoperability. No Razer code, binaries, images or firmware are included; the notes describe
observed protocol behaviour in our own words
([ADR-0001](vault/Decisions/ADR-0001%20Clean-room%20reverse%20engineering.md)).

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
