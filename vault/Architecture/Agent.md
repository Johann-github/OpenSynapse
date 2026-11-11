# Agent

The process `opensynapse-agent` owns all HID sessions. The CLI and GUI talk to it as clients.
Decisions: [[Decisions/ADR-0005 Agent IPC]], [[Decisions/ADR-0002 Lean by design]],
[[Decisions/ADR-0007 Per-app profiles]].

## Threads
No async runtime; every thread blocks in the kernel when idle.
- **Core actor:** owns the device list and all sessions and serves requests from a channel one
  at a time. HID access is serialised here and nowhere else.
- **Accept thread:** waits on the pipe and starts a connection thread. A `Waker` unblocks it on shutdown.
- **One thread per connection:**
  - Control: forwards each request to the core and writes the response.
  - Subscribe: writes the core's events to the client.
- **Hotplug:** the `CM_Register_Notification` callback (OS thread) posts "rescan" to the core.
  A burst of notifications becomes one rescan.
- **Foreground:** `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` out of context, on its own thread
  blocked in `GetMessageW`; posts `Foreground { exe }` to the core when the executable changes.
  Stopped with `PostThreadMessageW(WM_QUIT)`.

## Per-app profiles
Rules: `%APPDATA%\OpenSynapse\app-profiles.toml` (`--app-profiles <file>` overrides), parsed by
`opensynapse_core::AppProfiles`. The core picks rule → `default` → none for the foreground exe.
Same target as active → nothing. Razer software running → nothing written, error event. Applies
through the shared engine with volatile DPI stages and skip-unchanged, only to devices with the
profile's `[device] pid`. The values from before the first app profile are kept (first value wins)
and written back when no rule matches and there is no `default`. Details: [[Decisions/ADR-0007 Per-app profiles]].

## Connection roles
The role is fixed in the `Hello`/`Welcome` handshake, which also checks the protocol version.
- `Control`: strict request → response. Requests: `Ping`, `ListDevices`, `ReadSnapshot`, `Write`,
  `ApplyProfile` (slice 2), `Shutdown`, `ReloadAppProfiles`, `AppProfiles` (protocol 2).
- `Subscribe`: events from server to client only. Events: `DeviceArrived`, `DeviceRemoved`,
  `Battery`, `DpiChanged`, `Hardware`, `Error`, `ShuttingDown`, `AppProfile` (protocol 2).

One role per connection avoids overlapped I/O. Frames are a u32 LE length plus a versioned
binary codec, max 1 MiB (`opensynapse-ipc`).

## Security
- The pipe name contains the user SID. The DACL is protected and allows only the current user.
- `PIPE_REJECT_REMOTE_CLIENTS`. The client checks that the server process runs as the same user.
- `FILE_FLAG_FIRST_PIPE_INSTANCE` makes the agent single-instance.
- No network.
- Writes keep the GUI's guards (`opensynapse-device`): read old → write → read back, restore,
  and a confirm flag while Razer software runs.

## Budgets
Idle CPU ~0 % (no polling, no timers), < 15 MB RAM idle, IPC uses only std + `windows-sys`.

## Lifecycle
GUI-subsystem binary (no window), refuses an elevated token, started at login by the per-user
Run key or by `opensynapse agent start`. → [[Decisions/ADR-0006 Agent autostart]]

## Clients
- CLI: `opensynapse agent ping|devices|snapshot|apply|watch|stop|start|autostart|app-profiles [reload]`
  (`crates/opensynapse-cli/src/agent.rs`). `OPENSYNAPSE_AGENT_PIPE` points it at another pipe (tests).
- GUI: same `opensynapse-ipc::Client` (slice 3).
