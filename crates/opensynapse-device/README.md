# opensynapse-device

Device reads and guarded writes, generic over `opensynapse_hid::FeatureIo`. Shared by
the desktop GUI and the background agent; no OS code, no UI.

- `read_snapshot`: firmware first (fail fast), then every panel field with per-field errors.
- `guarded_write`: registry validation → Razer-software conflict gate → read old → write →
  read back. Left/right click need an explicit opt-in; "restore" writes back a read value.
- `profile_apply`: the one profile export/apply engine, used by `opensynapse profile
  export|apply` and by the agent's `ApplyProfile`. Validate → snapshot of what the
  profile overwrites → plan (`opensynapse-core`) → skip unchanged → write + read-back.
  No printing, no files: returns lines, typed errors and the restore `Profile`.
- Button editor model (`ActionKind`, `ActionEdit`, `key_choices`, `primary_guard`,
  `VIPER_BUTTONS`), shared by the GUI's main panel and per-app profile editor.
  `ActionEdit::from_action`/`to_action` convert profile actions through the core mapping
  (`opensynapse_core::plan::button_assignment`) and the export mapping
  (`profile_apply::button_action`).
- `Setting::Lighting` (extended 0x0F family only, [[Decisions/ADR-0014 Lighting]]): the
  canonical effect of one zone + optional raw brightness; written as 0F/02 then 0F/04,
  read back via 0F/82 + 0F/84 (`Effect::canonical` so equal modes compare equal).
  `validate` refuses devices without the extended family (standard 03/0A devices are
  CLI-only: no read-back), zones the registry does not list, brightness on
  `*NoBrightness` classes and, except for restores, effects the GUI does not offer.
  `lighting_setting` builds it from GUI choices. Untested on hardware.
- Report bytes come only from `opensynapse-protocol` builders.
- `fake` feature (and `cfg(test)`): `FakeMouse`, an in-memory Viper V3 HyperSpeed that
  answers feature reports, for tests in this crate and in dependent crates.

Lean by design: [[Decisions/ADR-0002 Lean by design]]. Vault: [[Architecture/Crates]]
