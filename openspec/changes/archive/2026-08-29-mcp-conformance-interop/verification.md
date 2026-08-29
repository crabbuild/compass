# Verification — MCP conformance and interoperability

Date: 2026-08-29

## Contract outcome

- Official current Model Context Protocol documentation identifies 2026-07-28
  as the latest published revision.
- The user explicitly selected the latest MCP revision as the governing C-010
  requirement.
- Compass retains 2026-only stdio and stateless HTTP lifecycles. OpenCode 1.18.25 HTTP
  remains recorded as incompatible because it sends the older 2025-11-25
  initialize/GET lifecycle; no legacy fallback was added.
- Exact passing discovery and real `graph_stats` invocation evidence remains
  recorded for Codex CLI 0.150.1 and Claude Code 2.1.251 on both transports and
  OpenCode 1.18.25 on stdio.

## New qualification artifacts

- `crates/compass-mcp/tests/protocol_conformance.rs`
- `scripts/qualify_mcp_conformance.sh`
- `scripts/qualify_codex_mcp_client.py`
- `docs/implementation/mcp-conformance-and-interop.md`

These new files are named explicitly and included in the isolated review packet
through a temporary Git index, leaving the user's real index untouched. Their
complete bodies are also covered by the deterministic gates below.

## Passing gates

- `openspec validate mcp-conformance-interop --strict`
- `cargo test -p compass-mcp --test protocol_conformance --locked`
- `cargo test -p compass-cli --locked deprecated_session_timeout -- --nocapture`
- `bash scripts/qualify_mcp_conformance.sh`
- `sh scripts/check_product_boundary.sh`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- `cargo test --workspace --lib --bins --locked`
- deterministic artifact-refiner manifest, file-integrity, constraint, and
  consistency checks
- final scoped adversarial review: PASS (0 critical, 3 warnings, 1 suggestion)
- adversarial-review sycophancy gate: PASS (score 0.0, strict mode)

The official HTTP runner passed every applicable check. Its check-granular
baseline contains only four fixture-tool diagnostics and one runner invocation
whose real Compass tool receives intentionally invalid arguments; transport,
wire-schema, discovery, routing, version negotiation, resource errors, and
DNS-rebinding checks are not baselined. The four diagnostic baseline entries
match runner check identifiers because the product does not advertise their
fixture-only tools or streams. The header entry is additionally cause-gated
after the run: it must be HTTP 400 / JSON-RPC `-32602` from the runner's invalid
`{}` tool arguments, so a header-mismatch regression cannot pass under that
identifier. It is not claimed as positive evidence for argument handling.
Native unit and end-to-end HTTP coverage separately pin optional-whitespace
normalization, including a successful real `graph_stats` invocation and
treatment of a whitespace-only header as missing.

## Final assessment

Completeness is 4/4 tasks and all MCP conformance requirements and scenarios
have implementation or qualification evidence. Correctness is covered by the
native stdio and transport tests (including legacy stdio rejection), the pinned
official HTTP runner, and the exact named-client matrix. Coherence matches the
design decision to implement only the latest published MCP revision. The clean
scoped final review and its strict anti-sycophancy gate both pass, so no critical
review finding remains before archive.
