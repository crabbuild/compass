# Refinement log — C-001

## Iteration 1 — 2026-08-28

Scope: `stream-snapshot-read`, validated against `.kbd-orchestrator/constraints.md`.

- Corrected the starting draft so `snapshot_reference` preserves corrupt-payload rejection through streaming validation.
- Added exact error-equivalence coverage for full read, stream, validation, and reference generation.
- Added consumer-stop, reopen, and interrupted-before-selector-swap coverage.
- Preserved the public `read_snapshot` signature and all serialized store contracts.
- Added the Unreleased changelog entry; no migration action is required.

Validation evidence:

- PASS — `cargo fmt --all -- --check`
- PASS — `cargo check --workspace --locked`
- PASS — `cargo test -p compass-store --locked` (18 tests)
- PASS — `cargo clippy -p compass-store --all-targets --all-features --locked -- -D warnings`
- PASS — `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- PASS — `cargo test --workspace --lib --bins --locked`
- PASS — `sh scripts/check_product_boundary.sh`
- PASS — `node scripts/check_viewer_assets.mjs`
- PASS — `openspec validate stream-snapshot-read --strict`
- PASS — `compass update .` (119,733 nodes, 281,337 edges; existing partial-graph warning reports 68 omitted edges and zero identity collisions)

Tooling note: the installed artifact-refiner adapter lacks its declared canonical controllers, JSON schemas, and `artifact-validator` agent. The KBD-defined deterministic fallback was used and this state was persisted explicitly; no unavailable validator is represented as having run. `npm ci` reported ten existing dependency advisories (one low, one moderate, eight high); C-001 changes no JavaScript or dependency manifests.

Overall: PASS — all applicable blocking constraints passed; proceed to adversarial diff review.

## Iteration 2 — 2026-08-28

Scope: reviewer-requested hardening of the final `stream-snapshot-read` diff.

- Added an explicit per-chunk `CHUNK_BYTES` rejection guard at the storage boundary.
- Added a raw-SQLite oversized-chunk regression that preserves the row digest so the
  test exercises the streaming bound itself.
- Reused the existing lowercase digest formatting path for incremental hashing.
- Refreshed the source hash in the QA receipt after the final code change.

Validation evidence:

- PASS — `cargo fmt --all -- --check`
- PASS — `cargo test -p compass-store --locked` (19 tests)
- PASS — `cargo clippy -p compass-store --all-targets --all-features --locked -- -D warnings`
- PASS — `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- PASS — `cargo test --workspace --lib --bins --locked`
- PASS — `sh scripts/check_product_boundary.sh`
- PASS — `node scripts/check_viewer_assets.mjs`
- PASS — `openspec validate stream-snapshot-read --strict`

Overall: PASS — the warning and suggestion from adversarial review iteration 1 are resolved.

## Iteration 3 — 2026-08-28

Scope: documentation and negative-path coverage requested by adversarial review iteration 2.

- Documented that streamed chunks remain provisional until successful terminal
  length and digest validation, with an explicit stage-then-commit pattern.
- Added direct, distinct regression coverage for payload-length mismatch and
  payload-digest mismatch.
- Recorded the generic callback-error suggestion as follow-up design work; it
  would expand the public error contract beyond this compatibility-preserving change.

Validation evidence:

- PASS — `cargo fmt --all -- --check`
- PASS — `cargo test -p compass-store --locked` (21 tests)
- PASS — `cargo clippy -p compass-store --all-targets --all-features --locked -- -D warnings`
- PASS — `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- PASS — `cargo test --workspace --lib --bins --locked`
- PASS — `sh scripts/check_product_boundary.sh`
- PASS — `node scripts/check_viewer_assets.mjs`
- PASS — `openspec validate stream-snapshot-read --strict`
- PASS — `compass update .` (119,741 nodes, 281,449 edges; existing partial-graph warning reports 68 omitted edges and zero identity collisions)

Overall: PASS — the final implementation and specification satisfy all applicable blocking constraints.

## Iteration 4 — 2026-08-28

Scope: allocation-bound enforcement requested by adversarial review iteration 3.

- Moved the corrupt-row chunk bound ahead of BLOB materialization using
  SQLite's BLOB-length metadata, with the bound repeated in the value query.
- Restored validated capacity reservation for the explicit full-read path while
  retaining one captured manifest generation.
- Extended the manifest-equivalence helper to cover node and edge counts.
- Confirmed SQLite BLOB-length semantics through current Context7 documentation.

Validation evidence:

- PASS — `cargo fmt --all -- --check`
- PASS — `cargo test -p compass-store --locked` (21 tests)
- PASS — `cargo clippy -p compass-store --all-targets --all-features --locked -- -D warnings`
- PASS — `openspec validate stream-snapshot-read --strict`
- PASS — `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- PASS — `cargo test --workspace --lib --bins --locked`
- PASS — `sh scripts/check_product_boundary.sh`
- PASS — `node scripts/check_viewer_assets.mjs`
- PASS — `compass update .` (119,752 nodes, 281,489 edges; existing partial-graph warning reports 68 omitted edges and zero identity collisions)

Overall: PASS — the final storage read path enforces its chunk bound before allocating chunk content.

## Iteration 5 — 2026-08-28

Scope: SQLite dynamic-typing and allocation-site hardening requested by adversarial review iteration 4.

- Reject non-BLOB snapshot chunks before the length probe and value read.
- Repeat the BLOB-type and byte-length predicates in the value query and retain
  a defensive Rust byte-length check.
- Added a multibyte TEXT corruption regression proving no chunk reaches the consumer.
- Repeated the graph-size cap immediately before full-read allocation.

Validation evidence:

- PASS — `cargo fmt --all -- --check`
- PASS — `cargo test -p compass-store --locked` (22 tests)
- PASS — `cargo clippy -p compass-store --all-targets --all-features --locked -- -D warnings`
- PASS — `openspec validate stream-snapshot-read --strict`
- PASS — `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- PASS — `cargo test --workspace --lib --bins --locked`
- PASS — `sh scripts/check_product_boundary.sh`
- PASS — `node scripts/check_viewer_assets.mjs`
- PASS — `compass update .` (119,755 nodes, 281,524 edges; existing partial-graph warning reports 68 omitted edges and zero identity collisions)

Overall: PASS — dynamic SQLite storage classes cannot bypass the chunk-byte contract.
