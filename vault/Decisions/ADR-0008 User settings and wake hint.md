# ADR-0008: User settings file and the "mouse asleep" hint

**Status:** accepted (2026-11-11)

**Context:** A wireless mouse that has gone to sleep does not answer configuration
commands. The user sees a raw transport error (`no answer after 20 attempts (last problem:
status Timeout)`), which does not say what to do. The user wants a friendly hint
("move the mouse to wake it"), but **only when switched on; default off**. That needs a
place for user preferences that both the CLI and the GUI read. Constraints:
[[Decisions/ADR-0002 Lean by design]] (no new crates, no watchers, no polling), single
source of truth per fact.

## Evidence: what a sleeping mouse produces

| Fact | Source | Confidence |
|---|---|---|
| After ~5 min idle the Viper V3 HyperSpeed (via dongle) did not answer; the error was "status Timeout, 20 attempts" both through the agent and directly. Moving the mouse fixed it. | [[RE-Log/2026-11-10 Agent slice 1 test]] (hardware) | observed |
| So the dongle stays reachable and answers the feature-report GET with status byte **0x04 (No Response / Timeout)**; the session resends (Synapse's rule for 0x04, [[Protocol/Report Format 2.5]]) until `max_out` (20) is used up. No busy loop, no I/O error, no hang. | same run + `Session::execute` | observed (the last attempt) / inferred (every attempt: only the last reason was recorded) |
| The mouse being out of range or switched off would look the same from the host (the dongle answers for it). | reasoning, no test | inferred |
| Which sleep setting triggers it: the 07/03 sleep timer (300 s on the test mouse). | timing matches | inferred |

## Decision

- **Typed class in the transport crate.** `opensynapse-hid` returns
  `HidError::NoResponse { attempts }` when every send was used up **and the last reply
  carried status Timeout (0x04)**. Any other exhaustion stays `HidError::Exhausted`. A
  Timeout followed by success is just success. The status byte itself is decoded in
  `opensynapse-protocol` (`Status::Timeout`); nothing re-derives bytes.
- **Text marker for string boundaries.** Many errors become strings (snapshot fields,
  write failures, profile-apply errors, IPC messages). The `NoResponse` text always
  starts with `opensynapse_hid::NO_RESPONSE` ("no response from the device"), and
  `opensynapse_hid::is_no_response_text(msg)` recognises it. One owner: `opensynapse-hid`.
- **IPC keeps the type.** New `ErrorKind::NoResponse` (wire tag 7, `PROTOCOL_VERSION` 3).
  The agent sets it for snapshot and apply failures of that class; the CLI keeps the kind in
  a typed `AgentError` so the class survives `?`. Write failures inside `Reply::Written`
  carry the class through the text marker.
- **Settings file:** `settings.toml` in the per-user config folder,
  `%APPDATA%\OpenSynapse\settings.toml` (same folder as `app-profiles.toml`, resolved by
  `opensynapse_sys::config`; elsewhere `$XDG_CONFIG_HOME/OpenSynapse` or
  `~/.config/OpenSynapse`). Format type `opensynapse_core::Settings` (pure, TOML):
  - missing file or missing key → default; unknown keys are tolerated **and written back
    unchanged** (a newer file survives a `set` from an older binary);
  - an invalid file is reported and never overwritten (CLI `set` fails, GUI checkbox is
    disabled with the reason); for the hint itself an unreadable file counts as "off";
  - writes go to a temp file and are renamed over the target.
  - Today one key: `wake_hint = false`. New settings are new `#[serde(default)]` fields
    plus a `Key` variant.
- **Who shows the hint:** only clients (CLI, GUI), never the agent. The hint text
  `opensynapse_core::WAKE_HINT` = "Mouse seems asleep — move it to wake it, then retry"
  is printed **in addition** to the raw error, only when `wake_hint = true` and the error is
  the `NoResponse` class.
- **CLI:** `opensynapse settings get`, `opensynapse settings set wake-hint on|off`. On a
  failing command the binary prints `Error: …` as before, then `Hint: …` when it applies.
- **GUI:** a "Settings" row with the checkbox "Show hint when the mouse is asleep"
  (module `crates/opensynapse-desktop/src/user_settings.rs`); the snapshot error and the
  "Write not done" line get the hint below them.
- The settings file is read when a command fails (CLI) or once at start (GUI); it is not
  watched. The agent does not read it.

## Consequences

- A sleeping mouse no longer burns the "last problem" text into the message; the error is
  "no response from the device (status Timeout after 20 attempts)".
- The GUI now depends on `opensynapse-core` (serde + toml, already in the workspace).
- Old clients (IPC v2) and the new agent refuse each other at the handshake; CLI, GUI and
  agent ship together.
- Open: auto-retry once the mouse wakes (would need a wake event; none identified yet,
  see [[Protocol/Hardware Events]]). Confirm with a capture that every attempt is 0x04.
