# Verification: stateless MCP HTTP

## Result

PASS for the MCP 2026-07-28 stateless HTTP cutover and CLI deprecation path.

## Evidence

- `cargo test -p compass-mcp --locked`: all 31 unit, integration, discovery,
  tool, resource, and raw-wire transport tests passed.
- The raw-wire HTTP regression proves `server/discover` advertises only
  2026-07-28, `tools/list` succeeds independently, no response contains
  `Mcp-Session-Id`, and missing/incomplete metadata, missing/mismatched method
  headers, and initialize fail with typed HTTP/JSON-RPC errors. A valid request
  carrying an obsolete session identifier remains independent and creates no
  session.
- The 64 MiB response boundary is restored for known and streaming bodies;
  known oversized bodies return distinct JSON-RPC error `-32023`, while an
  unknown-length multi-chunk regression proves the overflow chunk is replaced
  by an explicit `LengthLimitError`. Authenticated untrusted hosts are rejected
  before protocol diagnostics even when version metadata is missing or old.
  Existing bearer-token, API-key, request-limit, project-path, stdio, tool,
  resource, and rmcp 2.2 discovery-parity tests remain green.
- `cargo test -p compass-cli --lib --locked`: 79 passed.
- `cargo test -p compass-cli --test coverage_paths --locked`: 17 passed.
- `cargo test -p compass-cli --test mcp_http_cli --locked`: 1 passed and proves
  exact deprecation text plus unchanged exit codes 1 and 2.
- `cargo clippy -p compass-mcp --all-targets --all-features --locked -- -D
  warnings` and the equivalent `compass-cli` command passed.
- `cargo fmt --all -- --check`, workspace lib/bin Clippy, and workspace lib/bin
  tests passed under `--locked`.
- `cargo test -p compass-cli --test compass_product --locked`: 7 passed.
- `sh scripts/check_product_boundary.sh`: passed.
- `openspec validate mcp-stateless-http --strict` and `git diff --check` passed.
- Two adversarial rounds found four major issues; all were repaired and the
  focused plus full baselines were rerun before the final isolated packet.
- `compass update .`: 120,020 nodes, 282,421 edges, 3,460 communities, 68
  pre-existing omitted edges, and zero identity collisions.

Compatibility, migration, command, configuration, security, and release notes
record that no legacy MCP-2025 HTTP mode ships, `--stateless` is redundant but
accepted, and `--session-timeout` is removed in 0.5.0 after the 0.4.x warning
window.
