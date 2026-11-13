# opensynapse-ipc

Local IPC between the background agent and its clients (CLI, GUI). Decision:
[[Decisions/ADR-0005 Agent IPC]]. Does not touch HID; reuses `opensynapse-protocol`
value types only.

- `msg`: message types. Every connection starts with `Hello { version, role }` →
  `Welcome { version, agent_pid }`; the role (`Control` = request → response,
  `Subscribe` = server pushes events) is fixed for the connection's life.
- `codec`: hand-rolled, versioned binary encoding (std only). Little-endian integers,
  strict `bool` 0/1, `u32`-length strings (validated UTF-8) and lists (count never
  larger than the bytes left), `Option` 0/1, `Result` 0 Ok / 1 Err, an explicit tag
  per enum variant that is never reused. The decoder must consume the whole body.
  A layout change bumps `PROTOCOL_VERSION`. Version 3 added `ErrorKind::NoResponse` (tag 7:
  the device did not answer, e.g. a sleeping mouse; [[Decisions/ADR-0008 User settings and wake hint]]).
  Version 4 added `WireSetting::Lighting` (tag 5: raw 0x0F effect `profile, region,
  effect, flags, rate, colour list` + optional brightness; [[Decisions/ADR-0014 Lighting]]).
- `frame`: `u32` LE length + body, at most 1 MiB. Clean close → `UnexpectedEof`,
  oversize → `InvalidData`.
- `transport`: Windows named pipe `\\.\pipe\opensynapse-agent-<user SID>` with a
  protected DACL for the current user only, `PIPE_REJECT_REMOTE_CLIENTS`, and
  `FILE_FLAG_FIRST_PIPE_INSTANCE` (second agent → `AddrInUse`). The client checks that
  the server process runs as the same user (`PermissionDenied` otherwise) and connects
  with identification-only impersonation. No agent → `NotFound`. `Waker` unblocks a
  pending `accept` by connecting once. Unix: 0600 socket in `$XDG_RUNTIME_DIR`
  (compile-only for now). `unsafe` lives only in `transport/windows.rs`.
- `client`: `Client::connect[_to]` + `call` (incrementing ids, ids must match),
  `Client::subscribe[_to]` → `Subscription` iterator of events (ends on clean close).

Lean by design: [[Decisions/ADR-0002 Lean by design]] (no async runtime, no serde,
blocking I/O, one thread per connection on the agent side).
