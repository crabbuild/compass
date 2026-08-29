# Verification: vendor skip policy

## Result

PASS for the deliberate leave-and-explicitly-exclude policy.

## Evidence

- The focused vendor discovery/watcher regression passes, including every
  documented explicit exclusion surface.
- `cargo test -p compass-core --test vendor_policy --locked`: the build-level
  regression proves Rust and Go vendor sources are classified and published by
  default, then omitted by `.compassignore`, `extra_excludes` (the core form of
  CLI `--exclude`), and saved `BuildScope` exclusion.
- `cargo clippy -p compass-core --test vendor_policy --locked -- -D warnings`:
  passed.
- `cargo test -p compass-files --locked`: 43 tests passed across unit,
  contracts, and project-scope suites.
- `cargo clippy -p compass-files --lib --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --lib --bins --locked -- -D warnings`: passed.
- `cargo test --workspace --lib --bins --locked`: passed.
- `sh scripts/check_product_boundary.sh`: passed.
- `openspec validate vendor-skip-policy --strict`: passed.
- `git diff --check`: passed.
- `compass update .`: 119,880 nodes, 281,907 edges, 68 pre-existing omitted
  edges, zero identity collisions.

`cargo clippy -p compass-files --all-targets --all-features --locked -- -D
warnings` was also attempted. It reaches two pre-existing test-target lints in
`src/cache.rs` and `tests/contracts.rs` outside the new regression. The required
repository lib/bin Clippy baseline passes; this change does not alter or suppress
those unrelated all-target findings.
