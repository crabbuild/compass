# Verification

## Result

PASS. The `compass.code_context.v1` result envelope is implemented for the
four core navigation tools, preserves the exact prior `compass.query/1`
payload, and is ready to archive.

## Contract evidence

- Each tool advertises a closed draft-2020-12 output schema whose graph enum
  vocabulary is model-owned and whose 12 node-detail and 5 edge-detail variants
  are closed and exhaustively tested.
- `resultType: complete` remains the MCP protocol discriminator; `schema`
  carries the Compass envelope version.
- Repository/generation metadata comes from the exact query realization and is
  validated for both strict JSON graphs and immutable store metadata.
- Final encoded structured content honors `maxResponseBytes` and preserves the
  stable `query_response_too_large` identifier.
- Wire discovery pins output-schema digests and exact deprecation metadata.

## Compatibility evidence

- Four complete envelope goldens compare `data` exactly with independent
  pre-envelope `compass.query/1` fixtures.
- A separate pre-envelope golden preserves two parallel same-endpoint call
  occurrences with distinct identities, source sites, details, evidence, and
  deterministic order.
- Direction, ambiguity, warnings, bounds, truncation, and null pagination are
  covered. Remaining text-result tools are explicitly deprecated without
  removal or rename.

## Commands

- `cargo test -p compass-model --lib --locked` — passed (13 tests).
- `cargo test -p compass-graph --lib metadata_summary_rejects_empty_store_build_identities --locked` — passed.
- `cargo test -p compass-query --lib --locked` — passed (47 tests).
- `cargo test -p compass-mcp --locked` — passed (20 unit, 8 result-contract, 6 coverage-path, 2 discovery tests).
- `cargo clippy -p compass-model -p compass-graph -p compass-query -p compass-mcp --all-targets --all-features --locked -- -D warnings` — passed.
- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --lib --bins --locked -- -D warnings` — passed.
- `cargo test --workspace --lib --bins --locked` — passed.
- `cargo test -p compass-cli --test compass_product --locked` — passed (7 tests).
- `sh scripts/check_product_boundary.sh` — passed.
- `openspec validate mcp-result-envelope --strict` — passed.
- `git diff --check` — passed.
- `compass update .` — passed: 703 files, 120,215 nodes, 283,082 edges,
  3,385 communities, zero identity collisions; 68 pre-existing omitted edges.

## QA and adversarial review

Artifact Refiner used its deterministic fallback because the canonical runtime
is not installed. The final QA verdict is PASS. Six fresh-context adversarial
rounds drove all findings to closure; the final receipt is PASS with 0 critical,
0 major, 0 minor, and sycophancy score 0.0.
