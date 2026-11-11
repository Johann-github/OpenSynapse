# ADR-0007: Per-app profiles in the agent

**Status:** accepted (2026-11-11)

**Context:** Per-app profiles are the one feature that justifies an always-on agent
([[Decisions/ADR-0002 Lean by design]]). When the foreground application changes, the agent
should apply the profile mapped to that executable and switch back when the user leaves it.
Constraints: no polling, idle CPU ≈ 0 %, < 15 MB RAM, no new crates, all HID access through
the core actor ([[Architecture/Agent]]), no writes while Razer software runs, little flash wear.

**Decision:**
- **Foreground detection:** one thread (`foreground`, 256 KiB stack) installs
  `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)`
  and blocks in `GetMessageW`. The callback resolves window → `GetWindowThreadProcessId` →
  `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `QueryFullProcessImageNameW` and posts
  `CoreMsg::Foreground { exe }` only when the executable differs from the last one. It also
  reports the foreground executable once at start. Shutdown: `PostThreadMessageW(WM_QUIT)`,
  then the thread unhooks and is joined. A process the user may not query (e.g. some elevated
  ones) is ignored, so the previous profile stays. FFI lives in
  `crates/opensynapse-agent/src/foreground_windows.rs` (unsafe allowed there only);
  `windows-sys` features `Win32_UI_Accessibility` and `Win32_UI_WindowsAndMessaging` added.
  The watch runs even when no rules file exists, so `reload` can act on the app in front
  without a restart (cost: one wake-up per foreground change, nothing when idle).
- **Config:** `%APPDATA%\OpenSynapse\app-profiles.toml` (from the `APPDATA` variable; the
  agent flag `--app-profiles <file>` overrides it). Parsed in `opensynapse-core`
  (`AppProfiles`, pure, unit-tested):
  ```toml
  # profile paths relative to this file's folder or absolute
  default = "profiles/desktop.toml"   # optional: applied when no rule matches
  [[app]]
  exe = "cs2.exe"                      # file name, case-insensitive; or a full path
  profile = "profiles/cs2.toml"
  ```
  First matching rule wins; a pattern with `\` or `/` is compared with the whole path.
  Missing file = feature off (info log). Invalid file = error log, feature off, agent keeps running.
  Profile files are read at switch time, so edits to a profile need no reload.
- **Switching (core actor, `core/apps.rs`):**
  - Target = rule's profile, else `default`, else none. Same target as the active one → nothing
    (no read, no write, no event); alt-tab between unmatched apps is free.
  - Razer software running (`conflict()` at switch time) → nothing touched, `AppProfile` event
    with an error; the active profile stays, so the next focus change retries. Never forced.
  - A profile needs `[device] pid`; it is applied to every connected device with that PID.
    No such device → skipped (line "No connected device with PID …", nothing written).
  - The engine (`opensynapse_device::profile_apply::apply`) runs with
    `storage = Volatile`, `all = false`, `force = false`: validation, snapshot, skip-unchanged,
    read-back as for `profile apply`.
  - Each apply's restore profile is merged into one per device with "first value wins"
    (`Profile::fill_missing_from`), so it holds the values from before the *first* app profile.
  - No target and values kept → they are written back (same options) and forgotten. A device
    whose restore fails keeps its values for the next try; an unplugged device's are dropped.
  - Reload re-evaluates the app in front; an invalid or removed file therefore restores.
- **IPC** (`PROTOCOL_VERSION` 1 → 2, new tags only): `Request::ReloadAppProfiles` (6),
  `Request::AppProfiles` (7) → `Reply::AppProfiles(AppProfilesInfo)` (6) with file, enabled,
  parse error, default, rules, foreground exe, active profile, restore pending;
  `Event::AppProfile { exe, profile: Option<String>, lines, error }` (7) on every switch attempt
  (`profile: None` = values from before restored). CLI: `opensynapse agent app-profiles [reload]`;
  `agent watch` prints the event. The GUI ignores the event for now.

**Flash wear (what a switch writes persistently):**
| Setting | Storage on switch |
|---|---|
| DPI stages + active stage | **volatile** (storage byte 0, observed: [[RE-Log/2026-11-10 Button capture key and DPI]]); no flash write, a power cycle brings the stored stages back |
| Polling rate | persistent (no volatile variant known) |
| Sleep time | persistent (no volatile variant known) |
| Button assignments | persistent (onboard profile 1) |

Only settings whose value differs are written (skip-unchanged), and only on a change of the
target profile. Profiles that only set DPI stages therefore never touch flash. Users who switch
polling/sleep/buttons per app get one write per changed setting per switch; that is accepted
(it matches what Synapse does on profile switches, inferred) and documented here.

**Rejected:**
- Polling `GetForegroundWindow` on a timer: violates ADR-0002 (no polling).
- A hidden window + `WM_ACTIVATEAPP`/shell hooks: needs a window class; WinEvent out-of-context
  hooks need none and are delivered to our thread's message loop.
- Re-apply on every focus change: needless HID traffic and flash wear.
- Persistent DPI storage for switching: flash wear on every switch for no benefit.

**Consequences:**
- The agent links `user32` (working set 9.8 MB idle, measured; was 9.2 MB).
- A device that is plugged in while an app profile is active does not get it until the next
  switch (open).
- Unsupported-protocol devices with a matching PID get an error event, nothing written.
- Linux/macOS: the watch returns `Unsupported`; the feature is inactive there.

**Evidence:**
- Unit tests: `opensynapse-core` `app_profiles::tests` (parser, matching, path resolution, merge);
  `opensynapse-agent` `core::apps::tests` against fake mice (match → apply, same profile → no
  traffic, unmatched without default → restore and forget, default, Razer running → no traffic,
  unknown PID / missing PID → skip, DPI stages volatile, invalid/missing file, reload).
- Live (2026-11-11, release build, test pipe, rule `powershell.exe` → profile for PID 00C0 so no
  device write): foreground events for `Code.exe` at start, `powershell.exe` when a test window
  opened (switch in ~16 ms, "No connected device with PID 00C0; nothing written."), back to
  `Code.exe` when it closed ("Nothing to restore."). `reload` with an invalid file → error shown,
  CLI exit 1; valid file → "on, 1 rule(s)". Idle 20 s: CPU delta 0 s, working set 9.8 MB,
  private 2.5 MB, 5 threads. `agent stop` → clean exit 0.
