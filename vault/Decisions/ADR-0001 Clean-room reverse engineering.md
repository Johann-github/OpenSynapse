# ADR-0001: Clean-room reverse engineering

**Status:** accepted (2026-11-09)

**Context:** We reimplement Razer device configuration for interoperability.
Synapse 4 ships its protocol logic as minified JavaScript.

**Decision:**
1. RE material (extracted asar, cached bundles, decompiler output, captures) stays in
   `~/re-work/`, never in this repo.
2. The vault records *facts* about the protocol (byte layouts, IDs, value ranges,
   sequences) in our own words, with evidence + confidence.
3. Rust code is written from the vault notes, not from Razer code.
4. Device registry data contains only factual identifiers and capability values.
5. No firmware flashing, no DRM/licensing bypass, no Razer account APIs.

**Consequences:** slower than copying, but the repo is redistributable under MIT/Apache.
