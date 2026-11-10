# ADR-0005: Agent IPC = named pipe + own binary codec

**Status:** accepted (2026-11-10)

**Context:** Phase 4 needs a background agent (`opensynapse-agent`) that owns HID and serves
the CLI and GUI. [[Decisions/ADR-0002 Lean by design]] demands idle CPU ≈ 0 %, < 15 MB RAM,
no network, minimal dependencies.

**Decision:**
- **Transport (Windows):** named pipe `\\.\pipe\opensynapse-agent-<user SID>` via `windows-sys`
  (already a dependency, [[Decisions/ADR-0003 Own Windows HID backend]]). Protected DACL with a
  single allow entry for the current user, `PIPE_REJECT_REMOTE_CLIENTS`, and
  `FILE_FLAG_FIRST_PIPE_INSTANCE` for the single-instance check. The client verifies that the
  server process runs as the same user. Linux later: Unix socket, mode 0600.
- **Wire format:** length-prefixed frames (u32 LE, max 1 MiB) with a hand-rolled, versioned
  binary codec (std only, no serde_json). Profiles travel as TOML text.
- **Connection roles:** Control (request → response) or Subscribe (server → events only),
  fixed in the handshake. That avoids overlapped I/O.
- **Hotplug:** `CM_Register_Notification` for the HID interface class. No window and no polling;
  bursts are merged into one rescan.
- **Threads:** a core actor owns all HID sessions, one accept thread, and one thread per connection.
  No async runtime.
- **Events:** a pluggable `EventSource` trait. Slice 1 has none, because the Viper's event
  collection is still unknown ([[Protocol/Hardware Events]]).
- **Unsafe:** besides `opensynapse-hid/src/windows.rs`, also allowed in
  `opensynapse-hid/src/windows_hotplug.rs` and `opensynapse-ipc/src/transport/windows.rs`.

**Rejected:**
- Localhost TCP: any local process could connect, and ports can be squatted.
- stdin/stdout: only one client.
- serde_json: three extra crates.

**Consequences:**
- Writes over IPC use the same guarded semantics as the GUI (`opensynapse-device`):
  read old → write → read back, Restore, and a confirm flag while Razer software runs.
- The guarded-write logic moves from the GUI into the new `opensynapse-device` library.
