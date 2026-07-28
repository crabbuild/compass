# Compass code graph v1 qualification

Status: release gate
Contract: `compass.graph/1` and `compass.query/1`
Last fixture qualification: 2026-07-28

## Release claim

Compass publishes one strict, directed, multigraph NetworkX node-link envelope.
Pre-contract artifacts are not adapted or interpreted as an earlier v1: update
rebuilds them and read-only consumers return rebuild guidance. Program IR keeps
its independent `http://crab.build/compass/v1` artifact and is joined only by
the query engine.

The release gate checks:

- all 45 node kinds and 28 edge kinds have production producers;
- all 23 framework families have positive and near-match fixtures plus shared
  exact, ambiguous, unresolved, heuristic-wiring, and incremental-resolution
  coverage;
- heuristic graph edges retain a rule and an exact wiring site;
- cold, warm, restored, and checkout-root builds publish identical bytes;
- Rust, CLI, MCP, the viewer, and VS Code consume the fingerprinted
  `compass.query/1` enum/field manifest;
- source paths, atomic graph publication, SQLite recovery, cancellation, and
  VS Code packaging run on the supported platform matrix.

## Commands

Fixture and client qualification:

```bash
make qualify-code-graph-v1
```

Locked repository qualification:

```bash
./scripts/qualify_code_graph_v1.sh \
  --repositories tests/qualification/code-graph-v1-repositories.toml
```

The repository lock rejects symbolic refs and abbreviated object IDs. Each
entry pins a unique size/language cell, and every declared framework has at
least three named route-to-handler flows. The current polyglot lock uses an
immutable Compass corpus commit so every supported language and framework can
be exercised by the same current binary without network services or generated
source.

## Evidence interpretation

Qualification is fail-closed. A valid empty query remains a successful
`compass.query/1` response, but any unknown schema, unsafe source path,
unattributed heuristic edge, dangling endpoint, vocabulary producer gap,
framework gap, or byte mismatch exits non-zero.

The locked mode prints JSON Lines containing the Compass checkout revision,
repository revision, graph SHA-256, node/edge counts, cold and incremental
index time, and symbol-search latency. These records are run evidence, not a
portable performance promise; regressions are evaluated within the same CI
runner class.

## Recorded locked-corpus evidence

The 2026-07-28 macOS arm64 qualification of
`compass-polyglot-framework-corpus` at
`9b9c9f788856417331628119d3e594d6fa563f0d` produced:

- graph digest
  `sha256:2f228d55d4b7b70c54bcfac8eb3beb5055627789cc9f7262921eb41db7b59253`;
- 14,029 nodes and 41,628 edges across 25 emitted node kinds and 19 emitted
  edge kinds;
- 5,023 coverage records, 11 heuristic edges with wiring evidence, and six
  surfaced normalization diagnostics;
- 70 routes: 52 exact, 12 bounded ambiguous, and six unresolved; no exact route
  lacked a handler `routes_to` edge;
- all 69 declared framework flows had present source files, and all 23
  framework families emitted route nodes;
- byte-identical cold and unchanged incremental graphs;
- 44,826.57 ms cold indexing, 6,519.54 ms unchanged indexing, and 9,820.17 ms
  process-level CLI search latency on the qualification machine.

Ambiguous and unresolved routes are retained as explicit query evidence; they
are not counted as false exact resolution. The six diagnostics record
impossible structural self-loops that the publisher dropped rather than
silently representing as valid relationships.

## Platform matrix

Linux runs the complete fixture gate. macOS, Linux, and Windows run the
deterministic publication, query-index recovery, process cancellation, and
repository-contained source-navigation suites. The VS Code Electron test
verifies the public command inventory, while package and VSIX smoke tests
verify the shipped extension artifact.
