# Refinement log — C-003

## Iteration 1 — 2026-08-28

- Measured the real checkout and recorded its earlier stack-overflow failure.
- Built the 2,000-file deterministic equivalent and measured total bytes,
  payload composition, and full-build time.
- Implemented a root-relative, saturating, metadata-only estimate after
  verified-output reuse and before cold extraction.
- Added unit and end-to-end regressions for determinism, typed limit outcome,
  actionable text, oversized partial files, zero extraction progress, and the
  pinned 20% timing ceiling.

Validation: focused regressions, affected all-target/all-feature Clippy, strict
OpenSpec, public product tests, product boundary, workspace lib/bin Clippy and
tests, code-graph fixture qualification, formatting, and diff hygiene pass.
Compass graph refresh passes with 68 existing omitted edges and zero identity
collisions.

The broader `cargo test -p compass-core --locked` reproduces an unrelated
existing TypeScript star-reexport assertion (4 edges observed vs 2 expected).
The mandated workspace lib/bin baseline and every C-003-specific gate pass.

Tooling note: the installed artifact-refiner package lacks its declared
canonical controllers, schemas, and validator agent. The deterministic KBD
fallback was used; no unavailable validator is claimed. Independent K3 review
was dispatched with judge != producer, but both the initial call and its single
allowed retry returned no response within three minutes, so no verdict is
claimed.

Overall: PASS.
