# opensynapse-cli

CLI dispatch: `list`, `info`, `battery`, `dpi get|set`, `dpi-stages`, `polling get|set`,
`sleep get|set`. Global `--pid <hex>` and `-v` (log every report).

Writes are dry runs unless `--apply`; they read the old value first, write, read back,
and print the restore command. `--force` overrides range checks and the
Razer-software-running guard.

Vault: [[Architecture/Crates]], [[Status]]
