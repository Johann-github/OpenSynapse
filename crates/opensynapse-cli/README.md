# opensynapse-cli

CLI dispatch: `list`, `info`, `battery`, `dpi get|set`, `dpi-stages [get|set]`,
`polling get|set`, `sleep get|set`, `buttons get|set`, `profile export|apply`. Global
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

`--profile` defaults to 1. Key names: `a`-`z`, `0`-`9`, `enter`, `esc`, `backspace`,
`tab`, `space`, `f1`-`f12` (USB-HID usage IDs). Only these actions can be written yet;
if the old assignment is anything else, `set` prints its raw function/data instead of
a restore command.

Profiles (TOML, schema in `crates/opensynapse-core/README.md`):

    opensynapse profile export [--buttons 1-5] [--out FILE]
    opensynapse profile apply FILE [--volatile] [--apply] [--force]

`export` reads the DPI stages (persistent storage, incl. active stage), the polling
rate (auto variant, as `polling get`), the sleep time and the assignments of the
listed buttons (`1-5`, `1,2,4`, `1-3,6`; default `1-5`) on the normal and HyperShift
layer of onboard profile 1, and prints a profile (or writes it to `--out`). Values the
profile format cannot express (unknown button functions, unnamed keys, right-side
modifiers, X≠Y stage DPI, polling rates other than 125/500/1000) are left out and
listed raw on stderr. A failed read is also a stderr note.

`apply` parses and validates the file against the device's DPI range/step from the
device registry (all problems are printed; exit code is non-zero), checks
`[device] pid` and the registry's polling rates (`--force` overrides those two), then
reads every setting the profile will overwrite. Without `--apply` it prints each
planned write (old -> new value and the raw report). With `--apply` it first saves
those values to `<name>.restore.toml` next to the input (`-2`, `-3`, … if that file
exists; earlier snapshots are never overwritten) and prints

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

Vault: [[Architecture/Crates]], [[Status]]
