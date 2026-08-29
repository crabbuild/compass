# C-009 deterministic QA — iteration 3

## Focus

Golden outputs and semantic preservation.

## Evidence

- Four checked-in complete result fixtures pass for `search_symbols`,
  `get_callers`, `get_callees`, and `get_impact`.
- Each result retains the prior `compass.query/1` response under `data`.
- Tests cover directed caller/callee results, exact edge multiplicity in the
  fixture, explicit ambiguity, bounded truncation, warnings, null continuation,
  stable order, closed output schemas (including every tagged detail variant
  and graph enum), and MCP 2026 `resultType: complete`.
- Table-driven schema checks serialize all 45 node kinds, 13 node roles, 29
  edge kinds, 12 node-detail variants, and 5 edge-detail variants, including
  per-variant negative mutations.
- A separate pre-envelope golden pins two parallel call occurrences with
  distinct IDs, source sites, detail payloads, evidence, and stable order.
- The inner-pass/outer-fail response bound is exercised through rmcp
  `call_tool` and preserves `query_response_too_large`.
- Empty repository/generation identities fail graph validation before an MCP
  envelope is emitted.
- A low-level immutable-store snapshot regression proves empty identities are
  also rejected when metadata is read without materializing the complete graph.
- `cargo test -p compass-mcp --locked` passed: 20 unit, 8 result-contract, 6
  coverage-path, and 2 discovery tests.

## Verdict

PASS. The wrapper does not discard or reinterpret typed query evidence.
