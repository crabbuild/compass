# C-009 deterministic QA — iteration 2

## Focus

Implementation ownership, determinism, and boundedness.

## Evidence

- Envelope construction stays inside `compass-mcp`; query semantics remain in
  `compass-query` and typed records remain in `compass-model`.
- Metadata comes from the exact pinned graph or immutable store realization
  used by `CodeQueryEngine`, so an atomic replacement cannot skew the envelope
  identity away from its payload.
- One `compass-model` validator rejects empty source-tree/generation identities
  for strict JSON graphs, immutable store metadata summaries, and query-engine
  metadata from either backend.
- Evidence/confidence summaries use saturating counters and iterate only over
  bounded returned records.
- Tool discovery remains a fixed vector; local and rmcp wire goldens pin exact
  output-schema digests and deprecation metadata.

## Verdict

PASS. The MCP layer is thin and adds no unbounded read or nondeterministic map
iteration at the public contract boundary.
