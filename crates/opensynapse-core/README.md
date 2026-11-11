# opensynapse-core

Pure types and TOML config for OpenSynapse. No I/O.

Vault: [[Architecture/Crates]], [[Protocol/Commands 2.5]]

## Device profile

`Profile::from_toml(&str)` / `Profile::to_toml()` read and write a profile;
`Profile::validate(&DeviceLimits)` checks values and lists every problem with its
TOML path; `Profile::plan(&Target)` validates and returns the `PlannedWrite`s
(`Setting` + `opensynapse_protocol::Request`) that apply it. Requests come only from
`opensynapse-protocol` builders.

Every section is optional; a profile only sets what it lists.

```toml
[device]
pid = "00B8"                 # 4 hex digits (0x prefix allowed)

[dpi]
stages = [400, 800, 1600, 3200, 6400]   # same DPI for X and Y; 1..=max stages
active_stage = 2             # 1-based position in `stages`

[polling]
hz = 1000                    # 125, 500 or 1000

[power]
sleep_seconds = 300          # u16; unit inferred to be seconds

[buttons.4]                  # normal mode, key = button ID
action = "key"
key = "a"                    # a-z, 0-9, f1-f12, enter, esc, backspace, tab, space
mods = ["ctrl"]              # optional: ctrl, shift, alt, win

[buttons.5]
action = "mouse-button"      # also "double-click"; target 1 (left) ..= 8
target = 5

[hypershift.buttons.4]       # HyperShift mode (separate slot; absent = not written)
action = "dpi-up"
```

Device-specific limits (`DeviceLimits`: DPI min/max/step, max stages) are passed in by
the caller, usually from the device registry. `Target` also picks the onboard profile
slot for button writes, the DPI storage byte (default persistent) and the polling
command variant (period or HyperPolling).

## Per-app rules

`AppProfiles::from_toml(&str)` reads `app-profiles.toml` (used by the agent,
[[Decisions/ADR-0007 Per-app profiles]]); `profile_for(exe)` returns the first matching
rule's profile, else `default`; `resolve_profile_path` resolves it against the rules file's
folder. `Profile::fill_missing_from` merges restore profiles (first value wins).

```toml
# profile paths relative to this file's folder or absolute
default = "profiles/desktop.toml"   # optional: applied when no rule matches
[[app]]
exe = "cs2.exe"                      # file name, case-insensitive; or a full path
profile = "profiles/cs2.toml"
```
