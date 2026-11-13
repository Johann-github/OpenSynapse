# opensynapse-device-registry

Razer device identity and capabilities. Embeds `data/devices/*.toml` at compile time
(`include_str!`), parses once on first use, looks up VID/PID by primary PID, then
variant PIDs, then receiver PIDs.

`Device::hardware_tested()` is the single owner of "has OpenSynapse been tested on this
product with real hardware" (`tested = true` in the TOML; only the Viper V3 HyperSpeed so
far). The CLI and the desktop GUI use it to warn before writes; nobody hard-codes PIDs.

`Protocol::framing()` / `Protocol::is_supported()` are the single owner of "which protocol
families OpenSynapse speaks" (2.5 tested, 4.0 inferred; 3.0 and unknown not).
[[Decisions/ADR-0015 Protocol 4.0 and 3.0 support]]

Schema: `data/devices/SCHEMA.md`. Vault: [[Devices/Device Registry]], [[Architecture/Crates]]
