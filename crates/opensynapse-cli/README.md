# opensynapse-cli

CLI dispatch: `list`, `info`, `battery`, `dpi get|set`, `dpi-stages`, `polling get|set`,
`sleep get|set`, `buttons get|set`. Global `--pid <hex>` and `-v` (log every report).

Writes are dry runs unless `--apply`; they read the old value first, write, read back,
and print the restore command. `--force` overrides range checks and the
Razer-software-running guard.

Buttons (class 02, see [[Protocol/Commands 2.5]]):

    opensynapse buttons get <button> [--profile N] [--hypershift]
    opensynapse buttons set <button> mouse-button <n> [--profile N] [--hypershift] [--apply]
    opensynapse buttons set <button> double-click [n] [--profile N] [--hypershift] [--apply]

`--profile` defaults to 1. Only mouse-button and double-click can be written yet; if
the old assignment is anything else, `set` prints its raw function/data instead of a
restore command.

Vault: [[Architecture/Crates]], [[Status]]
