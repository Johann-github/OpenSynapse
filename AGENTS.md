# OpenSynapse — Agent Guide

Native, local-first Razer Synapse replacement in Rust. Windows first (dev hardware:
Razer Viper V3 HyperSpeed, PID 0x00B8), Linux/macOS second.

## Repo = Obsidian vault
- Research notes in `vault/`, linked with `[[wikilinks]]`. Every finding goes into a
  note **before** code depends on it. Start at `vault/Status.md` (handoff), then `vault/Home.md`.
- Each crate has a `README.md` that links to the vault notes it implements.
- Log RE work in `vault/RE-Log/YYYY-MM-DD <topic>.md` (use `vault/Templates/RE-Log.md`).
- Protocol facts: `vault/Protocol/`. Device facts: `vault/Devices/` + `data/devices/`.

## Clean-room rule (non-negotiable)
- Never copy Razer source (minified JS), binaries, images, or firmware into the repo.
- Describe behaviour in your own words: byte layouts, command IDs, ranges, sequences.
- RE working material lives outside the repo (`~/re-work/`), never under this folder.
- Each protocol claim cites its evidence (file/function name in Synapse, capture, or
  hardware test) and a confidence: `observed` / `inferred` / `unknown`.

## Lean by design (non-negotiable)
See `vault/Decisions/ADR-0002 Lean by design.md`. Only device configuration.
No ads/promos/store/news, no account/login, no cloud, no telemetry, no Electron/WebView.
No polling loops; idle CPU ~0 %. Every new dependency must be justified; prefer std.
Budgets: CLI < 5 MB, agent < 15 MB RAM idle, `opensynapse list` < 200 ms.

## Architecture
| Crate | Role |
|---|---|
| `crates/opensynapse` | CLI binary |
| `crates/opensynapse-cli` | CLI dispatch |
| `crates/opensynapse-core` | Pure types, config (TOML). No I/O |
| `crates/opensynapse-protocol` | Pure encode/decode of Razer reports. No I/O, fully unit-tested |
| `crates/opensynapse-device-registry` | Device identity + capabilities, loaded from `data/devices/` |
| `crates/opensynapse-hid` | hidapi transport, device sessions, retries |

Single source of truth: a protocol constant or decision has exactly one owner
(`opensynapse-protocol`). Other crates call it; they never re-derive bytes.

## Verify
- Inner loop: `cargo test -p <crate>` / `cargo check -p <crate>`.
- Before commit: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.
- Hardware-writing commands must be opt-in and restore-safe; never flash firmware.
