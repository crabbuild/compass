# Refinement log — C-002

## Iteration 1 — 2026-08-28

Scope: `actionable-snapshot-limit-error`, validated against `.kbd-orchestrator/constraints.md`.

- Centralized canonical graph oversize failures in a typed `SnapshotError::Limit` constructor.
- Kept a zero-byte manifest classified as corruption rather than conflating it with a resource limit.
- Named only shipped remediation controls: `--exclude <pattern>` and `.compassignore`.
- Added a real-binary regression that mutates a digest-consistent immutable SQLite snapshot and asserts stderr plus exit code 1.
- Withheld `COMPASS_MAX_GRAPH_BYTES` from the message until C-004 implements it on the publication path.
- Used the backend-neutral store contract; no dependency manifest or lockfile change remains.

Validation evidence:

- PASS — `openspec validate actionable-snapshot-limit-error --strict`
- PASS — `cargo fmt --all -- --check`
- PASS — focused graph limit classification test
- PASS — `cargo test -p compass-cli --test snapshot_limit_cli --locked`
- PASS — affected-crate all-target/all-feature Clippy with `-D warnings`
- PASS — `cargo test -p compass-cli --test compass_product --locked`
- PASS — `sh scripts/check_product_boundary.sh`
- PASS — `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- PASS — `cargo test --workspace --lib --bins --locked`
- PASS — `./scripts/qualify_code_graph_v1.sh --fixtures-only`
- PASS — `compass update .` (119,818 nodes, 281,678 edges; existing partial-graph warning reports 68 omitted edges and zero identity collisions)

Non-blocking baseline note: `cargo test -p compass-graph --locked` reaches and passes the changed unit test, then reproduces an unrelated existing failure in `tests/import_alias_identity.rs` because a framework import fixture lacks a direct-evidence source anchor. The mandated workspace lib/bin baseline and all C-002-specific/public gates pass.

Tooling note: the installed artifact-refiner adapter lacks its declared canonical controllers, JSON schemas, and `artifact-validator` agent. The KBD-defined deterministic fallback was used and persisted explicitly; no unavailable validator is represented as having run.

Overall: PASS — all applicable blocking constraints and C-002 contract evidence pass; proceed to adversarial diff review.

## Iteration 2 — 2026-08-28

Scope: first adversarial review and packet-scope correction.

- Clarified the recovery text so it is accurate during both failed publication
  and read-side validation of an existing immutable snapshot: retry or rebuild
  with a smaller scope.
- Documented why the shared manifest validator deliberately carries the same
  remediation in both contexts.
- Added the public compatibility classification to the changelog.
- Confirmed the original critical and unused-dependency findings were caused by
  the packet builder omitting the untracked CLI regression; the scoped packet
  is regenerated with that new file included.

Validation evidence:

- PASS — `cargo fmt --all -- --check`
- PASS — `openspec validate actionable-snapshot-limit-error --strict`
- PASS — focused graph limit classification test
- PASS — `cargo test -p compass-cli --test snapshot_limit_cli --locked`
- PASS — affected-crate all-target/all-feature Clippy with `-D warnings`

Overall: PASS — substantive review feedback is resolved and the review input now contains the complete C-002 diff.

## Iteration 3 — 2026-08-28

Scope: ownership and stable-contract hardening from the completed PASS review.

- Replaced raw SQLite table and namespace manipulation with the public
  backend-neutral `Store` interface.
- Added a graph-owned public manifest-key encoder and re-exported the existing
  snapshot layout constants instead of duplicating private storage layout.
- Removed the temporary `rusqlite` test dependency and lockfile change.
- Pinned the complete stable remediation message in the graph unit test.
- Confirmed `sha2` and `tempfile` were already declared `compass-cli`
  dependencies; the review warning about missing declarations was a false alarm.

Validation evidence:

- PASS — focused graph and CLI regressions
- PASS — affected-crate all-target/all-feature Clippy with `-D warnings`
- PASS — strict OpenSpec validation and diff hygiene

## Iteration 4 — 2026-08-28

Scope: final variant audit, helper scope, and platform portability.

- Audited every `SnapshotError::Corrupt` and `SnapshotError::Limit` use: no
  production caller branches on the reclassified variant; external matches are
  test assertions, while core/query wrappers remain variant-agnostic.
- Renamed the bounded digest helper to `digest_canonical_graph_json` and
  confirmed it has no selector, delta, or tree call sites.
- Passed the graph path to the CLI as an `OsStr` rather than lossy UTF-8.
- The fourth judge confirmation produced no response on its initial call or
  single allowed retry; no confirmation verdict is claimed. The last completed
  independent review was PASS (0 critical, 2 warnings, 1 suggestion), and all
  of those follow-ups are resolved with focused tests.

Final validation evidence:

- PASS — `cargo fmt --all -- --check`
- PASS — focused graph and real-binary CLI regressions
- PASS — affected-crate all-target/all-feature Clippy with `-D warnings`
- PASS — `cargo test -p compass-cli --test compass_product --locked`
- PASS — `sh scripts/check_product_boundary.sh`
- PASS — `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- PASS — `cargo test --workspace --lib --bins --locked`
- PASS — final-source `./scripts/qualify_code_graph_v1.sh --fixtures-only`
- PASS — `compass update .` (119,806 nodes, 281,681 edges; existing partial-graph warning reports 68 omitted edges and zero identity collisions)

Overall: PASS — final source satisfies all applicable blocking constraints.
