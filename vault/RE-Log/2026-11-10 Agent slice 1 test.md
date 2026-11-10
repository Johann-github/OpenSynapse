# 2026-11-10 Agent slice 1 test

**Question:** Does `opensynapse-agent` (slice 1) work end to end on the real mouse, within budget?
**Evidence (source / tool):** release build, Viper V3 HyperSpeed dongle, `opensynapse agent …` CLI, PowerShell `Get-Process`. → [[Architecture/Agent]], [[Decisions/ADR-0005 Agent IPC]]

## Findings
- (observed) `agent ping`, `agent devices`, `agent snapshot --pid 00B8` work over the named pipe
  `\.\pipe\opensynapse-agent-<SID>`. The snapshot has every field, including both button modes.
- (observed) A second `opensynapse-agent` exits with code 2: "already running (pid N)".
- (observed) `agent watch` showed exactly one `removed` and one `arrived` for one dongle unplug/replug
  (CM_Register_Notification, no polling).
- (observed) `agent stop` ends the agent and the watch stream ("Agent is shutting down."). After that,
  `agent ping` says "agent is not running".
- (observed) Budgets: working set 9.2 MB (target < 15 MB), private 2.0 MB, 4 threads, exe 1.0 MB.
  CPU time stayed 0.047 s over 169 s, including the replug: idle ≈ 0 %.
- (observed) After about 5 min idle the wireless mouse did not answer (status Timeout, 20 attempts),
  directly too. Moving the mouse fixed it. The device sleeps; not an agent bug.

## Open questions
- Handle "device asleep" nicely (clear error text; maybe a retry when the mouse wakes, needs events).
- Elevated agent vs. non-elevated client (untested).

## Next
- Slice 2: event collection discovery + HID input EventSource; ApplyProfile in the agent; GUI over IPC.
