# 2026-11-11 Button write storage (flash wear question)

**Question:** Does 02/0C (set button assignment) have a volatile variant, so per-app profiles can
change buttons without writing the mouse's onboard memory on every switch?
**Evidence (source / tool):** Synapse 4 code in `~/re-work/synapse4` (not in repo): rzDevice25
`setSingleButtonAssignment` in `bm/assets/index-42d42381.js`, its callers in the product UI
bundles `fetched/mw/13xx_main.js`. Builds on [[RE-Log/2026-11-11 Button capture off and DPI down]].

## Findings
- (observed, code) The set function packs `[firstArg, buttonId, mode, fnId, fnDataSize, data…]`,
  zero-pads to the command's data size and sends it; there is no separate storage byte.
- (observed, code) Callers pass the first argument as `obmId` (a mouse wrapper calls
  `setSingleButtonMapping(e.obmId, e.mapping)`), i.e. an onboard-memory profile slot, not a
  storage flag. In our captures Synapse always used 1 on the Viper (1 onboard slot).
- (observed, hardware) A write with slot 1 survives a power cycle (power-cycle test 2026-11-11).
- ~~(unknown) What slot 0 does.~~ **Resolved 2026-11-12 (observed, hardware): slot 0 is volatile.**
  See "Result" below.

## Test plan (needs the user for the power cycle; Synapse closed, agent stopped)
1. `buttons get 4 --profile 0` and `--profile 1` (read both slots).
2. `buttons set 4 off --profile 0 --apply`; read both slots again; user presses button 4 in a browser.
3. If it acts as "off": user switches the mouse off/on; read both slots; press again.
4. Volatile if step 3 shows button 4 back to mouse button 4 while slot 1 never changed.
5. Restore: `buttons set 4 mouse-button 4 --profile 1 --apply` (and `--profile 0` if it took).
If slot 0 is volatile: per-app profiles should write buttons to slot 0 (and restore = nothing
to write after a power cycle); the editor warning ([[Decisions/ADR-0007 Per-app profiles]]) can go.

## Result (2026-11-12, Viper V3 HyperSpeed 00B8 via dongle, Synapse closed, agent stopped)
1. Before: slot 0 and slot 1 both read button 4 = mouse button 4 (function 1, `[04]`).
2. `buttons set 4 off --profile 0 --apply` → accepted, read-back slot 0 = off (function 0),
   slot 1 still mouse button 4. User: button 4 does nothing in the browser (slot 0 acts at once).
3. User power-cycled the mouse (switch off ~3 s, on). Button 4 = browser back again.
   Read: slot 0 = mouse button 4, slot 1 = mouse button 4. DPI 800 unchanged.
4. **Conclusion (observed):** the first byte of 02/0C mirrors the DPI storage byte:
   **0 = volatile (current, lost on power-off), 1 = onboard slot (persistent)**. A slot-0 write
   overrides the active mapping without touching flash. Reading with 0 returns the active
   (volatile) mapping. No restore needed.
5. Consequence: per-app profiles can write buttons with slot 0 → no flash wear; the editor's
   flash warning becomes obsolete once the agent does that.
