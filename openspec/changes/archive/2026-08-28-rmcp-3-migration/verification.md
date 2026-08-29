# Verification: rmcp 3.1.4 migration

## Result

PASS for the dependency/API migration and rmcp 2.2 discovery parity.

## Evidence

- Root `Cargo.toml` pins `rmcp = "=3.1.4"`; Cargo metadata and `Cargo.lock`
  resolve only rmcp 3.1.4.
- `cargo test -p compass-mcp --locked`: 31 unit, transport, tool, resource,
  discovery, and duplex protocol tests passed.
- The `server_discover_matches_rmcp_2_2_golden` test performs a real
  `server/discover` exchange and matches server identity, capabilities, ordered
  tools and schema digests, and ordered resources to the pre-migration golden.
- `cargo clippy -p compass-mcp --all-targets --all-features --locked -- -D
  warnings`: passed.
- Temporarily installed `cargo-deny 0.20.2`; `cargo-deny check` reports
  advisories, bans, licenses, and sources all passing. Existing allowed
  duplicate-version warnings remain warnings under repository policy.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --lib --bins --locked -- -D warnings`: passed.
- `cargo test --workspace --lib --bins --locked`: passed.
- `cargo test -p compass-cli --test compass_product --locked`: 7 passed.
- `sh scripts/check_product_boundary.sh`: passed.
- `openspec validate rmcp-3-migration --strict`: passed.
- `git diff --check`: passed.
- `compass update .`: 119,961 nodes, 282,152 edges, 3,436 communities,
  68 pre-existing omitted edges, and zero identity collisions.

`cargo package -p compass-mcp --allow-dirty --locked` was also attempted. It
cannot package this workspace member because the internal `compass-core 0.3.7`
dependency is not present on crates.io; this is existing workspace publication
state and is unrelated to the rmcp migration.
