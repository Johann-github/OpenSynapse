# ADR-0002: Lean by design — no bloat, no ads

**Status:** accepted (2026-11-09)

**Context:** Synapse 4 is an Electron app (RazerAppEngine.exe alone is 212 MB, ~460 MB
install, several background processes, ~500 MB user-data cache). It ships ads/promos,
store links, recommendation engine, telemetry (Sentry, BigDataSDK), Razer ID login,
cloud sync, and many unrelated web apps.

**Decision — OpenSynapse ships only device configuration:**
- No Electron/WebView. Native Rust; GUI (later) native and optional.
- No ads, promotions, store, news, recommendations, gamification/rewards, Razer ID,
  account, cloud sync, telemetry, crash upload, auto-update pinging.
- No always-on background service unless a feature needs it (e.g. per-app profiles);
  then one small agent process, event-driven (no polling loops), idle CPU ≈ 0 %.
- Out of scope unless asked: Chroma Studio/visualizers, AI lighting, macros recorder UI,
  Loupedeck, IoT, game integrations, firmware updates.

**Budgets (enforced in CI later):**
| Metric | Target |
|---|---|
| CLI binary (release, stripped) | < 5 MB |
| Agent RAM idle | < 15 MB |
| Agent CPU idle | ~0 % (no timers when nothing changes) |
| Startup `opensynapse list` | < 200 ms |
| Runtime deps | minimal; every new crate justified in the PR |

**Build profile:** release uses `lto = "fat"`, `codegen-units = 1`, `opt-level = "s"`
(size first, measure), `strip = true`, `panic = "abort"` for binaries.
