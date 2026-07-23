# Query and navigate

Load this reference for codebase questions when a graph exists.

## Natural-language traversal

```bash
compass query "where is authentication enforced?"
compass query "payment retries" --dfs
compass query "payment retries" --budget 1500
compass query "payment retries" --context CheckoutService
```

The default traversal favors broad relevant context. Use `--dfs` when tracing a
specific chain. A token budget bounds rendered output; it does not change graph
contents.

Before retrying a weak result, derive a small vocabulary set from the request:
exact symbol spellings, file or crate names, domain nouns, and likely community
labels already present in `GRAPH_REPORT.md`. Retry with one concrete anchor at a
time. Do not add technologies or components unsupported by the repository.

Use a non-default graph or immutable commit explicitly:

```bash
compass query "authentication flow" --graph other/graph.json
compass query "authentication flow" --at HEAD~20
```

`--graph` and `--at` are mutually exclusive.

## Focused graph operations

```bash
compass explain PaymentGateway
compass path CheckoutHandler PaymentGateway
compass affected authorizePayment --depth 3
compass tree
```

- `explain` reports a matched node and connected context.
- `path` reports the shortest known graph route; preserve relation direction.
- `affected` follows impact relations and returns a review candidate set.
- `tree` combines repository structure with graph metadata.

If a label is ambiguous, retry with the exact node ID, symbol spelling, or source
file returned by `query`.

Use `--context VALUE` to anchor a common term inside a subsystem. Prefer a
shorter query plus an exact context over a long prose prompt containing several
unrelated questions. Split multi-part investigations so the evidence for each
claim stays attributable.

## Exact CompassQL

CompassQL is a deterministic, read-only openCypher subset. Use it for exact
patterns, parameters, stable JSON, or automation:

```bash
compass query --cql \
  "MATCH (caller)-[:CALLS]->(target)
   WHERE target.label = 'authorizePayment()'
   RETURN caller.id, target.id
   LIMIT 20"

compass query --cql \
  'MATCH (caller)-[:CALLS]->(target)
   WHERE target.label = $target
   RETURN caller.id' \
  --param target='authorizePayment()' \
  --format json
```

Use `PROFILE` only when query-plan details are needed. Run
`compass query --help` and consult the repository's CompassQL support document
before using syntax beyond known supported clauses or changing execution limits.

For reusable automation, prefer `--file`, `--params-file`, and JSON or JSONL
output over shell interpolation. Use parameters for values rather than splicing
untrusted text into CompassQL. Keep timeout, row, path-depth, expanded-relation,
and memory limits enabled; raise one only when the bounded query demonstrably
needs it. The REPL and stdin modes are interactive/input transports, not extra
query capabilities.

## Query Program IR

Use `compass program` when the question concerns normalized functions, call
evidence, or capability completeness rather than graph topology:

```bash
compass program coverage
compass program show <symbol-id>
compass program explain-call src/lib.rs:240
compass program query \
  "MATCH (f) WHERE f.kind = 'program_function' RETURN f.symbol_id, f.coverage"
```

The Program IR CompassQL projection is offline and read-only. Check the
capability state before using a result as change-impact evidence: `partial`,
`indeterminate`, and `failed` results require qualification or stronger
evidence. Function nodes expose `call_resolution_state` and
`impact_eligible`; only resolved targets create `CALLS` edges, and an
unresolved call never proves that no downstream target exists.

## Evidence discipline

Query output is scoped evidence, not a generated narrative. Verify material
claims against `source_file` and `source_location`. Distinguish:

- a direct extracted relation,
- a resolved or inferred relation with confidence,
- an ambiguous candidate,
- no path represented in the current graph.

Do not translate “no result” into “impossible.” Check graph freshness and query
spelling first.
