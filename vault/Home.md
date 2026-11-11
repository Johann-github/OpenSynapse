---
tags: [moc]
---
# OpenSynapse Vault

Entry point. Repo root is the vault; code lives in `crates/`.

## Maps
- [[Status]] — **current phase, next steps, handoff**
- [[Backlog]] — parked items for later
- [[Architecture/Crates]] — crate map, where code lives
- [[Protocol/Overview]] — Razer HID protocol families (2.5 / 3.0 / 4.0)
  - [[Protocol/Report Format 2.5]] · [[Protocol/Commands 2.5]] · [[Protocol/Protocol 3.0 and 4.0]] · [[Protocol/Hardware Events]] · [[Protocol/Windows Transport]]
- [[Devices/Device Registry]] — supported devices, PIDs, capabilities
- [[Architecture/Synapse 4 Anatomy]] — how Synapse 4 is built (RE target map)

## Process
- [[Decisions/ADR-0001 Clean-room reverse engineering]]
- [[Decisions/ADR-0002 Lean by design]]
- [[Decisions/ADR-0003 Own Windows HID backend]]
- [[Decisions/ADR-0006 Agent autostart]]
- [[Decisions/ADR-0007 Per-app profiles]]
- [[Decisions/ADR-0008 User settings and wake hint]]
- RE-Log: [[RE-Log/2026-11-09 Target survey]] · [[RE-Log/2026-11-09 Protocol 2.5]] · [[RE-Log/2026-11-09 Protocol 3.0 4.0 and events]] · [[RE-Log/2026-11-09 Windows transport]] · [[RE-Log/2026-11-09 Device database]] · [[RE-Log/2026-11-09 Hardware read test]] · [[RE-Log/2026-11-09 Button remap capture]]
- Templates: [[Templates/RE-Log]], [[Templates/Protocol Command]], [[Templates/Device]]
