# ADR-0011: Linux autostart = XDG autostart entry, install to `~/.local/bin`, refuse root

**Status:** accepted (2026-11-13). Stage 4 of [[Architecture/Linux and macOS port]].
Confidence: **inferred**. Unit-tested on Windows and type-checked + clippy-clean for
`x86_64-unknown-linux-gnu`; not run on a Linux machine yet.

**Context:** On Windows the agent starts at login through the HKCU Run key
([[Decisions/ADR-0006 Agent autostart]]) and `opensynapse install` copies the binaries to
`%LOCALAPPDATA%\Programs\OpenSynapse` and writes a Start-menu shortcut
([[Decisions/ADR-0009 Install location]]). On Linux all of this was `Unsupported`.
[[Decisions/ADR-0002 Lean by design]]: no service manager requirement, no new crate, no polling.
The CLI (`agent autostart`, `install`, `uninstall`) and the GUI ("Start agent at login") should
work unchanged on top of the same `opensynapse_sys` API.

**Options (autostart):**
| | What | For | Against |
|---|---|---|---|
| a | XDG autostart entry `$XDG_CONFIG_HOME/autostart/opensynapse-agent.desktop` | every XDG desktop session (GNOME, KDE, Xfce, …; systemd ≥ 246 also turns these entries into units through `systemd-xdg-autostart-generator`); starts **inside the graphical session**, where logind's `uaccess` ACL on the hidraw nodes applies ([[Architecture/Linux and macOS port#Linux setup]]) and where a later X11 foreground watch (stage 6) needs `DISPLAY`; one plain text file | no automatic restart after a crash; does nothing on a bare TTY/SSH login |
| b | systemd user unit `~/.config/systemd/user/opensynapse-agent.service` + `systemctl --user enable` | restart on failure, `journalctl --user` | needs systemd; `default.target` of the user manager can start before (or without) the graphical session, so the seat ACL / `DISPLAY` are not guaranteed; enabling means running `systemctl` (a subprocess) or writing the `wants` symlink by hand; two files |

**Decision:**
- **Autostart: (a), the XDG autostart entry.** File `opensynapse-agent.desktop` in
  `$XDG_CONFIG_HOME/autostart` (an absolute value only, per the XDG Base Directory spec), else
  `~/.config/autostart`. Contents: `Type=Application`, `Name`, `Comment`, `Exec=<agent>`,
  `Terminal=false`, `X-GNOME-Autostart-enabled=true`. Written as temp file + rename
  (`config::write`). `agent autostart off` deletes the file; a missing file is "off".
  An entry the desktop switched off (`Hidden=true` or `X-GNOME-Autostart-enabled=false`) reads
  as off; turning autostart on rewrites our file. Only our own file name is ever touched.
- **`Exec=` quoting** (Desktop Entry spec, "The Exec key"): `%` → `%%`; an argument with a
  reserved character (space, tab, newline, `"` `'` `\` `>` `<` `~` `|` `&` `;` `$` `*` `?` `#`
  `(` `)` `` ` ``) is put in double quotes, and inside them `"` `` ` `` `$` `\` get a backslash.
  Then the general string escapes apply (`\\`, `\n`, `\t`, `\r`), so a literal backslash in a
  quoted path is written as four backslashes. Reading reverses both layers and takes the first
  argument. The "command" the CLI/GUI show and compare (`autostart::get`, `points_to`) is the
  `Exec` value with the string escapes undone; paths compare case-sensitively on Linux.
- **Install location: `~/.local/bin`** (`$HOME/.local/bin`; there is no XDG variable for it).
  It is the per-user executables folder of the systemd file-hierarchy convention and is on
  `PATH` in common distributions, so `opensynapse` works from a terminal after install, which
  matters more on Linux where the CLI is a main interface. Rejected: `$XDG_DATA_HOME/opensynapse/`
  with nothing on `PATH` (the user would have to edit `PATH` or call a long path). Because the
  folder is shared, uninstall deletes only our three binaries and their `.tmp` / `.old`
  leftovers and **never removes the folder** (`install::DEFAULT_DIR_SHARED`); a `--dir` folder is
  still removed when empty. Replacing a running binary needs no `.old` on Linux (rename over a
  running file is allowed; the old inode lives on until the process exits).
- **GUI launcher** (the Start-menu shortcut's equivalent): default-location installs write
  `$XDG_DATA_HOME/applications/opensynapse.desktop` (else `~/.local/share/applications`) with
  `Name=OpenSynapse`, `Exec=<gui>` (quoted as above), `TryExec=<gui>`, `Path=<install dir>`,
  `Categories=Settings;HardwareSettings;`; no icon yet. Temp file + rename. Uninstall (default
  location) deletes exactly this file. `--dir` installs never touch it, as on Windows.
- **Refuse root:** the agent exits with code 3 (`agent_exit::ELEVATED`) when its effective uid
  is 0, mirroring "refusing to run as administrator" on Windows; `--allow-elevated` overrides.
  A root agent would serve every local client with root rights, and with the udev rule root is
  never needed. The euid comes from the `Uid:` line of `/proc/self/status` (real, **effective**,
  saved, fs), read with std: `opensynapse-sys` has no `libc` in its dependency tree (it is
  pulled in only by `opensynapse-hid` through hidapi), and adding it for one `geteuid` call is
  not worth a dependency. An unreadable `/proc` counts as "not root" (`unwrap_or(false)`), as on
  Windows.
- **Code:** pure, platform-independent `opensynapse_sys::desktop_entry` (rendering, quoting,
  parsing our entry back, XDG folder resolution from given env values, `/proc` status parser);
  thin `#[cfg(target_os = "linux")]` glue in `lib.rs` (`autostart::{get, enable, disable}`,
  `is_elevated`); `install::dir` = `~/.local/bin` on Linux; `shortcut::{launcher_path,
  launcher_bytes, MENU_NAME}` dispatch between `.lnk` and `.desktop`. macOS stays `Unsupported`
  (stage 5: LaunchAgent).

**Consequences:** no new dependency, no new process at runtime, no polling. Autostart only
works in a graphical XDG session; headless users can start the agent from their own unit
(documented here, not shipped). `Process::open` is still Windows-only, so on Linux `install`
asks a running agent to stop but cannot wait for its exit; the new agent may then find the old
socket still bound (exit 2) in a short race. Open item for a hardware session.

**Evidence:** unit tests `desktop_entry::tests` (folders from env values incl. relative/blank
XDG values, quoting round trip with space `"` `` ` `` `$` `\` `%`, string escapes, autostart
entry round trip, disabled/foreign entries, launcher text, euid parser) and `install::tests`
(`dir_from_home`; Linux-only `linux_autostart_plans_use_exec_quoting`, run by CI).
`cargo clippy -p opensynapse-sys --target x86_64-unknown-linux-gnu --all-targets -D warnings`
clean on 2026-11-13. Spec rules from the freedesktop Desktop Entry Specification and the XDG
Autostart / Base Directory specifications (inferred, no Linux session tested).
