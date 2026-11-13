# opensynapse-cli

CLI dispatch: `list`, `info`, `report`, `battery`, `dpi get|set`, `dpi-stages [get|set]`,
`polling get|set`, `sleep get|set`, `buttons get|set|list`, `lighting get|set`, `profile export|apply`, `agent ping|devices|snapshot|apply|watch|stop|start|autostart|app-profiles [reload]`, `install`, `uninstall`, `udev-rule` (prints the Linux udev rule to stdout and install steps to stderr; never writes). Global
`--pid <hex>` and `-v` (log every report).

Writes are dry runs unless `--apply`; they read the old value first, write, read back,
and print the restore command. `--force` overrides range checks and the
Razer-software-running guard.

Before a write command (`dpi set`, `dpi-stages set`, `polling set`, `sleep set`,
`buttons set`, `lighting set`, `profile apply`, `agent apply`; dry run and `--apply`) to a device the
registry does not mark as hardware-tested (`Device::hardware_tested`), one line goes to
stderr: `Warning: <name> has not been tested with OpenSynapse yet; writes may not work as
expected. Reads are safe. Please report: opensynapse report` The write is not blocked and exit codes are unchanged. `info`
prints `Tested with OpenSynapse: yes|no`.

DPI stages (class 04, 04/86 get, 04/06 set, see [[Protocol/Commands 2.5]]):

    opensynapse dpi-stages [get] [--persistent]
    opensynapse dpi-stages set --active <N> [--persistent] [--apply] [--force]

`get` (also the default) lists the stages and marks the active one with `*`. `set`
reads the stages from the same storage, changes only the active stage (`N` = position
in the list, 1 = first; usually equal to the stage ID), writes the whole list back and
verifies it. `dpi set` does not move the active stage, so use this to select one.
Whether selecting a stage also applies its DPI is unknown; `set` prints the current
DPI (04/85) before and after the write so you can see it. It never writes 04/05.

Lighting ([[Protocol/Lighting 2.5]], [[Decisions/ADR-0014 Lighting]]; **untested on
hardware**, no RGB device is `tested = true`, so the untested warning precedes every write):

    opensynapse lighting get [--zone Z] [--persistent]
    opensynapse lighting set off|spectrum [--zone Z] [--brightness PCT] [--persistent] [--apply] [--force]
    opensynapse lighting set static 00ff00 [--zone logo] [--brightness 80] [--apply]
    opensynapse lighting set breathing ff0000 [00ff00] [...]
    opensynapse lighting set wave [--direction 1|2] [...]

The family comes from the registry (`Device::lighting_family`); devices without one
(non-Chroma like the Viper, unknown driver classes, laptops) are refused. Zones: names
(`all`, `wheel`, `logo`, `backlight`, `underglow`, `side-stripe`, …) or IDs; a zone the
registry does not list for the device needs `--force`. Default zone `all` (0); reads of
`all` use the first registry zone.
- Extended (0x0F) devices: storage byte 0 (volatile) unless `--persistent`. `set` reads
  the old effect and brightness, writes 0F/02 (+ 0F/04), reads back and prints the
  restore command. If the old effect cannot be read, `--apply` needs `--force`.
- Standard (03/0A) devices: one effect for the whole device, no storage byte, no read
  command, so no read-back or restore of the effect; `--brightness` uses 03/03 on the
  zone's LED (backlight for `all`) and is read back.

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
    opensynapse install [--dir D] [--no-autostart]
    opensynapse uninstall [--dir D]

These commands never open HID themselves (they are dispatched before the HID
transport is created). `devices` lists what the agent sees, with the control path
for `--path`. `snapshot` reads all settings the panel shows (firmware, serial, mode,
battery, DPI, stages with `*` on the active one, polling, sleep, buttons in normal and
HyperShift mode); a value that could not be read prints as `n/a (reason)`. Without
`--pid`/`--path` it uses the only connected device. `watch` blocks on the event
subscription and prints one line per event (no timers) until the agent shuts down.
`stop` asks the agent to exit. If the agent's pipe does not exist the CLI prints
"OpenSynapse agent is not running (start it with `opensynapse agent start`)".

`install` copies CLI, agent and GUI from this build to `%LOCALAPPDATA%\Programs\OpenSynapse`,
stopping a running agent first and starting the installed one after; autostart is pointed at the
installed agent, and (default location only) a Start-menu shortcut `OpenSynapse` for the GUI is
written. `uninstall` reverses it and keeps the settings in `%APPDATA%\OpenSynapse`
([[Decisions/ADR-0009 Install location]]).
Linux ([[Decisions/ADR-0011 Linux autostart and install]], untested on a Linux machine): the
folder is `~/.local/bin`, the GUI gets an app-menu launcher
`~/.local/share/applications/opensynapse.desktop` (`$XDG_DATA_HOME`), and uninstall deletes only
our files there and keeps the shared `~/.local/bin` folder.

`start` launches the installed agent (else the one in the CLI's folder) detached (no window) and
waits up to 3 s for it to answer; exit code 3 from the agent means it refused to run
elevated (Linux: as root). `autostart` shows, adds or removes the per-user Run-key entry
([[Decisions/ADR-0006 Agent autostart]]; Linux: the XDG autostart entry
`~/.config/autostart/opensynapse-agent.desktop`, `$XDG_CONFIG_HOME`); it reports when the entry
points at another binary. `off` does not stop a running agent.

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

Device report ([[Devices/Device Registry]], section "Community device reports"):

    opensynapse [--pid HEX] report

Prints a Markdown block for a "Device report" issue: OpenSynapse version, OS, and per
connected Razer PID the registry entry (name, kind, protocol, tested), the HID collections
(interface, `MI_xx COLyy`, usage page/usage, report lengths, readable), whether the
protocol 2.5 control collection opened, and the result or error of each read: firmware,
serial, device mode, battery, charging, DPI, DPI stages (volatile, `*` = active), polling
rate, sleep timer, buttons 1-5 (profile 1, normal mode). Read-only: only GET commands, sent
to protocol 2.5 PIDs only (3.0, 4.0 and unknown PIDs list their collections, no reads); never writes, never
uses the network. Serials are masked (last 4 characters), raw HID paths are not printed.
Each read uses 5 sends instead of 20, so a sleeping device does not stall the report; if a
read gets no response the report says to wake the device and run it again.

Vault: [[Architecture/Crates]], [[Status]]
