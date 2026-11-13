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
`Device::lighting_family()` is the single owner of "which lighting commands does this
device answer" (extended 0x0F, standard 0x03, or none), picked from the Synapse driver
class; `lighting_zones()` returns the manifest zone IDs and `lighting_brightness()` is
false for the `*NoBrightness` classes. Implements [[Protocol/Lighting 2.5]],
[[Decisions/ADR-0014 Lighting]].

Schema: `data/devices/SCHEMA.md`. Vault: [[Devices/Device Registry]], [[Architecture/Crates]]
