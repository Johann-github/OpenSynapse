# opensynapse-device-registry

Razer device identity and capabilities. Embeds `data/devices/*.toml` at compile time
(`include_str!`), parses once on first use, looks up VID/PID by primary PID, then
variant PIDs, then receiver PIDs.

Schema: `data/devices/SCHEMA.md`. Vault: [[Devices/Device Registry]], [[Architecture/Crates]]
