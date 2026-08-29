# Decisions — C-001 refinement

## 2026-08-28 — Preserve corrupt-reference semantics

Use one chunk-streaming validation pass for `snapshot_reference` rather than a manifest-only read. This retains the prior rejection behavior for missing or modified payload chunks while removing the payload-sized allocation.

## 2026-08-28 — Preserve the compatibility collector

Keep `read_snapshot() -> Result<(SnapshotManifest, Vec<u8>), StoreError>` unchanged and implement it over the streaming API. The full allocation remains explicit and opt-in; production callers use bounded paths.

## 2026-08-28 — Accept deterministic validation fallback

The installed artifact-refiner package is incomplete. Persist the KBD constraint results and the tooling degradation instead of claiming schema/agent execution that the host cannot perform.

## 2026-08-28 — Keep the callback error contract compatible

The second adversarial review suggested a generic consumer error type. C-001 is explicitly additive and compatibility-preserving, and introducing a public streaming error wrapper would broaden its contract and migration surface. Keep the fallible `StoreError` callback for this change; record a generic callback error as follow-up design work if a production caller demonstrates that need.

## 2026-08-28 — Enforce the chunk bound before BLOB materialization

Use SQLite's `length(value)` metadata path before selecting a chunk BLOB, then repeat the bound in the value query. Current SQLite documentation confirms that BLOB length can be determined without reading the complete content. This makes the corruption boundary allocation-safe while retaining normal row-digest validation. The compatibility collector now preallocates from the already-validated manifest and streams that same captured generation.

SQLite's dynamic storage classes require an additional rule: snapshot chunks must remain BLOB values. Reject TEXT or other classes before the size probe, repeat the type predicate in the bounded value query, and retain a defensive Rust byte check. Repeat the graph-size cap immediately before full-read allocation even though manifest validation already enforces it.
