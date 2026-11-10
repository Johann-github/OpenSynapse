# opensynapse-device

Device reads and guarded writes, generic over `opensynapse_hid::FeatureIo`. Shared by
the desktop GUI and the background agent; no OS code, no UI.

- `read_snapshot`: firmware first (fail fast), then every panel field with per-field errors.
- `guarded_write`: registry validation → Razer-software conflict gate → read old → write →
  read back. Left/right click need an explicit opt-in; "restore" writes back a read value.
- Report bytes come only from `opensynapse-protocol` builders.
- `fake` feature (and `cfg(test)`): `FakeMouse`, an in-memory Viper V3 HyperSpeed that
  answers feature reports, for tests in this crate and in dependent crates.

Lean by design: [[Decisions/ADR-0002 Lean by design]]. Vault: [[Architecture/Crates]]
