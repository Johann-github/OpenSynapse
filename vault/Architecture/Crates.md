# Crates

| Crate | Note | Role |
|---|---|---|
| `opensynapse` | [[crates/opensynapse/README]] | CLI binary |
| `opensynapse-cli` | [[crates/opensynapse-cli/README]] | CLI dispatch |
| `opensynapse-core` | [[crates/opensynapse-core/README]] | Pure types + TOML config |
| `opensynapse-protocol` | [[crates/opensynapse-protocol/README]] | Report encode/decode, see [[Protocol/Overview]] |
| `opensynapse-device-registry` | [[crates/opensynapse-device-registry/README]] | See [[Devices/Device Registry]] |
| `opensynapse-hid` | [[crates/opensynapse-hid/README]] | hidapi transport |
| `opensynapse-sys` | [[crates/opensynapse-sys/README]] | Autostart (Run key), elevation, console attach ([[Decisions/ADR-0006 Agent autostart]]), per-user config folder ([[Decisions/ADR-0008 User settings and wake hint]]) |
| `opensynapse-desktop` | [[crates/opensynapse-desktop/README]] | egui GUI ([[Decisions/ADR-0004 GUI toolkit egui]]) |

Planned (mirrors OpenLogi): `opensynapse-agent` (background service owning HID),
`opensynapse-ipc`.
