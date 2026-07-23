# Query engine implementation

Compass has two complementary query paths: focused discovery/traversal and the
CompassQL compiler/executor. Both operate over the same indexed graph model.

> **Who this page is for:** contributors changing query ranking, traversal,
> CompassQL syntax/plans/execution, or machine output.
>
> **You will learn:** loading, indexes, discovery scoring, traversal, affected,
> CompassQL compilation and execution, caching, limits, profiling, and tests.
>
> **Prerequisites:** [Graph model](../concepts/graph-model.md) and
> [Workspace tour](workspace-tour.md).
>
> **Reading time:** 15 minutes.

## Load once into the query model

`GraphDocument::load`:

- requires the normal JSON extension;
- enforces the graph size cap;
- reuses a binary cache only when its signature matches;
- retains unknown document fields and attributes.

`Graph::from_document`:

- consolidates duplicate node IDs by extending attributes;
- ensures edge endpoints can be indexed;
- preserves parallel edges for multigraph documents;
- builds ID lookup;
- builds incoming/outgoing adjacency;
- builds `QueryIndex` and a schema fingerprint.

Read commands can force stored direction when compatibility documents require
it.

## Focused discovery path

```text
question
  -> query_terms()
  -> score_nodes()
  -> choose anchors
  -> query_graph_text() with BFS/DFS and budget
  -> focused text subgraph
```

### Text normalization

`compass-query::text`:

- normalizes labels and questions;
- removes common question noise;
- produces search tokens;
- normalizes context filters.

This is deterministic token matching, not embedding inference.

### Scoring

`score_nodes` uses indexed label/attribute evidence to rank candidates.
`find_node` and `pick_scored_endpoint` support commands that need one entity.

Ranking changes can alter which subgraph users see even when graph data is
unchanged. Protect them with realistic query fixtures and stable tie behavior.

### Traversal

`query_graph_text` traverses from anchors using:

- BFS or DFS;
- depth/working budget;
- optional relation context filters;
- hub avoidance/bounds;
- directed adjacency.

The renderer labels nodes and edges with provenance and source context.

## Explain and path

`render_explanation` separates incoming from outgoing relations and reports
degree. Human output may list a multigraph neighbor once while retaining
parallel edge degree.

`render_shortest_path` preserves stored arrow direction:

```text
A --calls--> B
```

When queried in reverse, the rendered arrow points backward rather than
pretending the relation reversed.

Path tests must cover:

- forward/reverse rendering;
- no path;
- repeated labels;
- directed/legacy graphs;
- parallel edges;
- tie ordering.

## Affected projection

Impact analysis needs only:

- node ID/label/location;
- edge endpoints and relation.

`GraphDocument::load_for_affected` can use a compact binary cache and avoids
retaining irrelevant attributes. `affected_nodes` traverses incoming
impact-relevant relation families under a depth.

This specialized projection improves cold performance without changing the
full graph/query model.

## CompassQL compile path

`compass-cypher` compiles:

```text
bytes
  -> tokens with spans
  -> AST
  -> semantic scope/type checks
  -> logical operators
  -> optimizer records
  -> LogicalPlan
```

Compile limits bound:

- source bytes;
- token count;
- nesting;
- path depth.

Diagnostics carry stable codes and byte spans. Unsupported read/write syntax
is rejected during compile/semantic analysis.

## CompassQL execution

`compass-query::cql` executes a logical plan over `Graph`.

Execution supports:

- indexed node candidates;
- directed expansion and bounded paths;
- repeated-variable joins;
- optional matching;
- correlated one-level existence checks;
- unwinding;
- filters and expressions;
- projection, distinct, union, order, skip, limit;
- aggregation;
- shortest-path families;
- typed values.

Runtime limits track:

- deadline/cancellation;
- returned rows;
- expanded relationships;
- working memory;
- path depth.

Limit checks occur during work, not only after a huge result is constructed.

## Plan cache

The plan cache key includes:

```text
exact source
language/planner versions
ordered parameter types
planning limits
graph schema fingerprint
```

It excludes:

- parameter values;
- graph values;
- result rows;
- deadline and cancellation state.

This allows safe plan reuse without leaking prior execution data.

## Result model

`QueryResult` contains:

- versioned schema;
- column metadata;
- typed rows;
- optional explain plan;
- optional profile.

Renderers produce:

- table;
- `compass.cql.result/1` JSON;
- `compass.cql.jsonl/1` header/rows/summary.

Output-to-file uses atomic completion. A failed execution must not leave a
valid-looking partial result.

## Explain and profile

`EXPLAIN` returns logical operators and optimization records without executing.

`PROFILE` adds per-operator/clause:

- input/output rows;
- candidate nodes;
- expanded relationships;
- working-memory estimates;
- elapsed time;
- cancellation checkpoints.

Profiles are part of performance diagnosis. They must not include parameter
values or secrets.

## Historical graph loading

`--at REV` is resolved in the CLI/history service:

1. resolve exact commit;
2. open history;
3. find and validate preferred realization;
4. materialize synchronously if missing;
5. reconstruct `GraphDocument`;
6. build the same `LoadedGraph`/query indexes.

The query engine does not need separate semantics for current and historical
graphs after loading.

## Test map

### `compass-query`

Cover normalization, scoring, traversal, direction, affected, execution,
limits, caching, profiles, and output values.

### `compass-cypher`

Cover lexer/parser/semantic/plan diagnostics, support records, optimization,
and language-version behavior.

### CLI tests

`compassql_cli.rs` and read-command tests cover:

- option sources;
- stdin/file/REPL;
- parameters;
- stdout/stderr/exits;
- output atomicity;
- `--graph`/`--at`;
- JSON/JSONL schemas.

### TCK and differential tests

The repository carries an attributed subset of the openCypher TCK and optional
Neo4j differential evidence. New support should update the matrix and add
portable feature coverage.

### Benchmarks

`scripts/benchmark_compassql.sh` measures compile/plan, cache hits, fixed
matches, paths, aggregation, optional matching, cancellation latency,
expansions, rows, and memory.

## Change checklist

When adding CompassQL syntax:

1. lexer/token/spans;
2. AST and parser;
3. semantic scope/type rules;
4. plan operator;
5. optimizer effect;
6. bounded execution;
7. typed result;
8. diagnostic code for invalid forms;
9. support matrix;
10. TCK/unit/CLI/differential/benchmark evidence.

When changing discovery ranking:

1. realistic question tokens;
2. multilingual/noise behavior;
3. stable ties;
4. hub/context filtering;
5. result budget;
6. compatibility snapshots;
7. cold/warm query performance.

## Related pages

- [CompassQL concepts](../concepts/compassql.md)
- [CompassQL reference](../COMPASSQL.md)
- [Graph model](../concepts/graph-model.md)
- [Performance qualification](../../PERFORMANCE.md)

**Next step:** run a CompassQL query with `EXPLAIN`, then locate each logical
operator in `compass-cypher` and its executor in `compass-query`.
