# opensynapse-device-registry

Razer device identity and capabilities. Embeds `data/devices/*.toml` at compile time
(`include_str!`), parses once on first use, looks up VID/PID by primary PID, then
variant PIDs, then receiver PIDs.

`Device::hardware_tested()` is the single owner of "has OpenSynapse been tested on this
product with real hardware" (`tested = true` in the TOML; only the Viper V3 HyperSpeed so
far). The CLI and the desktop GUI use it to warn before writes; nobody hard-codes PIDs.

Schema: `data/devices/SCHEMA.md`. Vault: [[Devices/Device Registry]], [[Architecture/Crates]]
