## 1. Snapshot Read Contract

- [x] 1.1 Complete the manifest-only and chunk-streaming APIs, retain the full-read signature, and route validation/reference production callers through one streaming verification pass; verify with focused valid and corrupt snapshot tests

## 2. Durability and Compatibility Coverage

- [x] 2.1 Add or strengthen round-trip, reopen, corruption/interruption, consumer-error, and active-publication atomicity tests; verify with `cargo test -p compass-store --locked`

## 3. Documentation

- [x] 3.1 Correct the store allocation contract and add a release-visible `CHANGELOG.md` entry, confirming that no `MIGRATION.md` action is required because the existing public signature is unchanged

## 4. Verification

- [x] 4.1 Run `cargo fmt --all -- --check`, `cargo test -p compass-store --locked`, and `cargo clippy -p compass-store --all-targets --all-features --locked -- -D warnings`
- [x] 4.2 Run the repository Rust baseline, applicable product-boundary gate, and `compass update .`; record any check or graph-refresh failure before certification
