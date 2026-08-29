## Why

C-014 can publish canonical generations to SurrealDB, but Compass cannot yet use
that projection for bounded graph-native reads or prove that those reads preserve
the current engine's semantics. C-015 adds the closed read surface and evaluates
it against the pre-ratified C-013 equivalence and product-value gates.

## What Changes

- Add parameter-bound, generation-pinned Surreal reads for exact symbol context,
  impact, directed path, and connected-subgraph operations.
- Add an engine-neutral structural-query result contract and a current-engine
  route so identical requests can be evaluated against both realizations.
- Enforce independent depth, node, edge, path, candidate, response-byte, and
  pagination bounds before and during traversal.
- Add deterministic semantic-corpus and scale-sample differential tests that
  treat any identity, direction, multiplicity, evidence, ordering, bound, or
  pagination mismatch as a failure.
- Measure every applicable C-013 gate on the same runner and apply the recorded
  falsifier protocol if a threshold fails; do not tune the corpus or threshold
  after observing results.
- Preserve the default dependency boundary and expose no caller-authored
  SurrealQL path.

## Capabilities

### New Capabilities

- `surreal-native-query`: Closed, bounded native Surreal graph reads and deterministic dual-engine equivalence qualification.

### Modified Capabilities

- None.

## Impact

The primary implementation lives in `compass-graphdb-surreal`; the comparison
route and shared structural contract live at the `compass-query`/`compass-model`
boundary. Qualification extends `benchmarks/qualification/` without changing
default CLI or MCP behavior. SurrealDB remains exact-versioned and feature-gated,
and default Compass builds remain free of Surreal dependencies.
