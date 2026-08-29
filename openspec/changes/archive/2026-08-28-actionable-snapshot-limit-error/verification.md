# Verification: actionable snapshot publication limit error

## Result

PASS. All requirements and scenarios are implemented with direct unit and
binary-level evidence. No migration is required.

## Requirement evidence

- Oversized canonical manifests return `SnapshotError::Limit`; zero-byte
  manifests remain `SnapshotError::Corrupt`.
- The complete stable remediation names `--exclude <pattern>` and
  `.compassignore`, omits `COMPASS_MAX_GRAPH_BYTES`, and is pinned verbatim in a
  `compass-graph` unit test.
- The released `compass` binary opens a digest-consistent oversized immutable
  snapshot through the store engine, writes no stdout, renders both remediation
  controls on stderr, and exits with code 1.
- The CLI fixture uses graph-owned snapshot layout helpers and the
  backend-neutral `Store` contract; it does not depend on raw SQLite schema.
- A repository-wide `SnapshotError` audit found no production consumer that
  branches on `Corrupt` versus `Limit`; the intentional reclassification does
  not change fallback, quarantine, or cleanup flow.

## Verification performed

- `cargo fmt --all -- --check`
- focused `compass-graph` limit test
- `cargo test -p compass-cli --test snapshot_limit_cli --locked`
- affected-crate all-target/all-feature Clippy with `-D warnings`
- `cargo test -p compass-cli --test compass_product --locked`
- `sh scripts/check_product_boundary.sh`
- `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- `cargo test --workspace --lib --bins --locked`
- `./scripts/qualify_code_graph_v1.sh --fixtures-only`
- `openspec validate actionable-snapshot-limit-error --strict`
- `compass update .`
- deterministic artifact-refiner fallback and independent adversarial diff
  review: PASS (0 critical, 2 warnings, 1 suggestion); all findings resolved

The broader `cargo test -p compass-graph --locked` command also reached and
passed the changed unit test, then reproduced an unrelated existing failure in
`tests/import_alias_identity.rs` involving a direct-evidence source anchor. The
mandated workspace lib/bin baseline and every C-002-specific/public gate pass.

An optional judge confirmation after resolving the PASS review's final findings
produced no response on its initial call or single allowed retry, so no
confirmation verdict is claimed.
