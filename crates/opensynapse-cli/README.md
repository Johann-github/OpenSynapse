# opensynapse-cli

CLI dispatch: `list`, `info`, `battery`, `dpi get|set`, `dpi-stages [get|set]`,
`polling get|set`, `sleep get|set`, `buttons get|set|list`, `profile export|apply`, `agent ping|devices|snapshot|apply|watch|stop|start|autostart|app-profiles [reload]`. Global
`--pid <hex>` and `-v` (log every report).

Writes are dry runs unless `--apply`; they read the old value first, write, read back,
and print the restore command. `--force` overrides range checks and the
Razer-software-running guard.

DPI stages (class 04, 04/86 get, 04/06 set, see [[Protocol/Commands 2.5]]):

    opensynapse dpi-stages [get] [--persistent]
    opensynapse dpi-stages set --active <N> [--persistent] [--apply] [--force]

`get` (also the default) lists the stages and marks the active one with `*`. `set`
reads the stages from the same storage, changes only the active stage (`N` = position
in the list, 1 = first; usually equal to the stage ID), writes the whole list back and
verifies it. `dpi set` does not move the active stage, so use this to select one.
Whether selecting a stage also applies its DPI is unknown; `set` prints the current
DPI (04/85) before and after the write so you can see it. It never writes 04/05.

Buttons (class 02, see [[Protocol/Commands 2.5]]):

    opensynapse buttons get <button> [--profile N] [--hypershift]
    opensynapse buttons set <button> mouse-button <n> [--profile N] [--hypershift] [--apply]
    opensynapse buttons set <button> double-click [n] [--profile N] [--hypershift] [--apply]
    opensynapse buttons set <button> key <name> [--mods ctrl,shift,alt,win] [--profile N] [--hypershift] [--apply]
    opensynapse buttons set <button> dpi-up [--profile N] [--hypershift] [--apply]
    opensynapse buttons list [--profile N] [--hypershift] [--offset N]

`--hypershift` selects mode 1 (HyperShift) instead of 0 (normal). Each button has a
separate slot per mode on the Viper V3 HyperSpeed: a HyperShift write leaves the
normal mapping untouched (hardware test). Factory defaults are the same in both
modes. `get` shows the requested mode; with `-v` it also prints reply byte 2, whose
meaning is unknown (it is not the mode). `list` reads 02/8E ("Get Button Assignment
List") once and prints one line per button, decoded like `get` (record layout
inferred); `-v` also prints the raw reply as hex.

`--profile` defaults to 1. Key names: `a`-`z`, `0`-`9`, `enter`, `esc`, `backspace`,
`tab`, `space`, `f1`-`f12` (USB-HID usage IDs). Only these actions can be written yet;
if the old assignment is anything else, `set` prints its raw function/data instead of
a restore command.

Profiles (TOML, schema in `crates/opensynapse-core/README.md`):

    opensynapse profile export [--buttons 1-5] [--out FILE]
    opensynapse profile apply FILE [--volatile] [--all] [--apply] [--force]

`export` reads the DPI stages (persistent storage, incl. active stage), the polling
rate (auto variant, as `polling get`), the sleep time and the assignments of the
listed buttons (`1-5`, `1,2,4`, `1-3,6`; default `1-5`) in normal and HyperShift mode
of onboard profile 1, and prints a profile (or writes it to `--out`). To keep the
file short, a `[hypershift.buttons.N]` entry is only written when it differs from
`[buttons.N]` (or when the normal mapping cannot be expressed). A missing HyperShift
entry therefore means "same as normal at export time"; applying the file leaves that
HyperShift slot untouched, so if you later change `[buttons.N]` and want HyperShift
to follow, add the HyperShift entry yourself. Values the
profile format cannot express (unknown button functions, unnamed keys, right-side
modifiers, X≠Y stage DPI, polling rates other than 125/500/1000) are left out and
listed raw on stderr. A failed read is also a stderr note.

`apply` parses and validates the file against the device's DPI range/step from the
device registry (all problems are printed; exit code is non-zero), checks
`[device] pid` and the registry's polling rates (`--force` overrides those two), then
reads every setting the profile will overwrite (each button slot in the mode the
profile writes; the snapshot is never shortened like an export). A planned write
whose setting already has the target value is skipped (less flash wear, fewer
round trips): the current value is re-encoded into a request and compared with the
planned request (command and arguments; transaction ID and CRC are not part of it).
A setting that could not be read or encoded counts as changed. Skipped settings print
as `unchanged: <setting> (<value>)`, followed by `N write(s), M unchanged.`; `--all`
writes them anyway (the old behaviour). If nothing differs, apply says so, writes
nothing and creates no restore file. Without `--apply` it prints each remaining
planned write (old -> new value and the raw report). With `--apply` it first saves
the old values of exactly the settings it will write to `<name>.restore.toml` next to
the input (`-2`, `-3`, … if that file exists; earlier snapshots are never
overwritten) and prints

    To restore: opensynapse --pid 00B8 profile apply <name>.restore.toml --apply

then writes in plan order (DPI, polling, sleep, normal buttons, HyperShift buttons),
reading each setting back and checking it re-encodes to the request sent; it stops at
the first failure and prints the restore hint again. Polling uses the same variant
detection as `polling set` (period 00/05, HyperPolling 00/40 if that is "not
supported"). If an old value the profile overwrites cannot be expressed in the
snapshot (or the snapshot would not validate), `--apply` refuses unless `--force`.
`--volatile` writes the DPI stages to storage byte 0 (the snapshot reads the same
storage; add `--volatile` when restoring).

Example profile:

```toml
[device]
pid = "00B8"

[dpi]
stages = [400, 800, 1600, 3200, 6400]
active_stage = 2

[polling]
hz = 1000

[power]
sleep_seconds = 300

[buttons.4]
action = "key"
key = "a"
mods = ["ctrl"]

[buttons.5]
action = "double-click"
target = 1

[hypershift.buttons.4]
action = "dpi-up"
```

Agent (talks to a running `opensynapse-agent` over the per-user named pipe, see
[[Decisions/ADR-0005 Agent IPC]] and [[Architecture/Agent]]):

    opensynapse agent ping
    opensynapse agent devices
    opensynapse [--pid HEX] agent snapshot [--pid HEX | --path P]
    opensynapse agent watch
    opensynapse agent stop
    opensynapse agent start
    opensynapse agent autostart [on|off]
    opensynapse [--pid HEX] agent apply FILE [--path P] [--all] [--apply] [--force]
    opensynapse agent app-profiles [reload]

These commands never open HID themselves (they are dispatched before the HID
transport is created). `devices` lists what the agent sees, with the control path
for `--path`. `snapshot` reads all settings the panel shows (firmware, serial, mode,
battery, DPI, stages with `*` on the active one, polling, sleep, buttons in normal and
HyperShift mode); a value that could not be read prints as `n/a (reason)`. Without
`--pid`/`--path` it uses the only connected device. `watch` blocks on the event
subscription and prints one line per event (no timers) until the agent shuts down.
`stop` asks the agent to exit. If the agent's pipe does not exist the CLI prints
"OpenSynapse agent is not running (start it with `opensynapse agent start`)".

`start` launches `opensynapse-agent.exe` from the CLI's folder detached (no window) and
waits up to 3 s for it to answer; exit code 3 from the agent means it refused to run
elevated. `autostart` shows, adds or removes the per-user Run-key entry
([[Decisions/ADR-0006 Agent autostart]]); it reports when the entry points at another
binary. `off` does not stop a running agent.

`agent apply` runs the same engine as `profile apply` (`opensynapse_device::profile_apply`)
inside the agent: same checks, skip-unchanged, read-back, same lines (DPI stages always
persistent). `--force` passes the Razer-software gate and the PID / rate / snapshot checks.
The agent returns the restore profile as text; this command saves it as
`<name>.restore.toml` (same naming as `profile apply`), after the writes rather than
before them, and still saves it when a write fails part-way.

`agent app-profiles` shows the per-app rules file, whether it is loaded (or why not), the
rules, the foreground executable and the active profile. `agent app-profiles reload` re-reads
the file and applies what it says for the app in front; it exits non-zero when the file is
invalid. `agent watch` prints each switch (`app profile <file> for <exe>`, then the engine's
lines). `OPENSYNAPSE_AGENT_PIPE` selects another agent pipe (tests).
See [[Decisions/ADR-0007 Per-app profiles]].

Settings ([[Decisions/ADR-0008 User settings and wake hint]]):

    opensynapse settings get
    opensynapse settings set wake-hint on|off

`settings.toml` lives in `%APPDATA%\OpenSynapse` (shared with the GUI). `get` lists every
setting and the file; a missing file means defaults. `set` refuses to overwrite a file it
cannot parse. With `wake-hint on`, a command that fails because the device did not respond
(direct HID `HidError::NoResponse`, agent `ErrorKind::NoResponse`, or that error's text in a
write failure) prints `Hint: Mouse seems asleep — move it to wake it, then retry` after the
usual `Error: …`. Default: off.

Vault: [[Architecture/Crates]], [[Status]]
