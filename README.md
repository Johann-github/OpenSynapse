# OpenSynapse

**A native, local-first alternative to Razer Synapse, written in Rust 🦀**
Configure DPI, polling rate, lighting, battery, and button remapping on Razer devices.
No account, no cloud, no telemetry, plain-TOML config.

> Status: **early research / pre-alpha.** The protocol is being reverse engineered
> from Synapse 4 for interoperability. See [[vault/Home|the research vault]].

Inspired by and structured after [OpenLogi](https://github.com/AprilNEA/OpenLogi)
(the same idea for Logitech HID++ devices).

## Repository = Obsidian vault

Open this folder in Obsidian. Research notes live in `vault/`, code in `crates/`,
every crate has a `README.md` that links into the vault so the graph view connects
protocol notes to the code that implements them.

| Path | Content |
|---|---|
| `vault/Home.md` | Entry point of the vault |
| `vault/Protocol/` | Wire protocol notes (report layout, command tables) |
| `vault/Devices/` | Per-device / per-family notes |
| `vault/RE-Log/` | Dated reverse-engineering log entries |
| `vault/Architecture/` | Crate map and runtime design |
| `vault/Decisions/` | ADRs |
| `data/devices/` | Machine-readable device registry (facts only: IDs, ranges, features) |
| `crates/` | Rust workspace |

## Crates

| Crate | Role |
|---|---|
| `opensynapse` | CLI binary (`opensynapse`) |
| `opensynapse-cli` | CLI dispatch |
| `opensynapse-core` | Pure types: config, device model, actions. No I/O |
| `opensynapse-protocol` | Pure Razer report encode/decode (protocol 2.5 / 3.0 / 4.0) |
| `opensynapse-device-registry` | VID/PID → device identity, protocol, capabilities |
| `opensynapse-hid` | Host transport (hidapi) + device sessions |

## Build

```sh
cargo build
cargo run -- list
```

## Legal

OpenSynapse is an independent project, not affiliated with or endorsed by Razer Inc.
"Razer" and "Synapse" are trademarks of Razer Inc. Reverse engineering is done
solely for interoperability. No Razer code, binaries, or assets are included in
this repository — notes describe observed protocol behaviour in our own words.
See [[vault/Decisions/ADR-0001 Clean-room reverse engineering]].

Licensed under either of MIT or Apache-2.0 at your option.
