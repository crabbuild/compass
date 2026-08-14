# Query engine implementation

Compass has two complementary query paths: focused discovery/traversal and the
CompassQL compiler/executor. Both operate over the same indexed graph model.

## Graph-engine boundary and query model

`compass-query` exposes an immutable `GraphEngine` boundary with two supported
implementations: `JsonGraphEngine` for the permanent compatible JSON artifact
and `StoreGraphEngine` for a validated `compass-store` snapshot. The store
engine reads the active Phase 2 graph-index snapshot, validates its manifest
and typed `store.ref` when opening a published SQLite generation, and pins the
resulting projection to that immutable realization. Library callers can use
`open_with_store` with any common-contract adapter, including the optional redb
adapter, without adding that backend to the CLI. The typed query algorithms
consume the same backend-neutral projected operations, so selection cannot
change ranking, direction, multiplicity, source anchors, truncation, or public
result schemas. `StoreGraphEngine` pins the selected manifest and directly uses
node, edge, metadata/file, name, term, incoming, and outgoing immutable roots.
It does not export or clone a complete `GraphDocument`, and it does not build a
second SQLite query index. Whole-graph consumers outside this typed command
family may still request a bounded materialized export.

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
- builds `QueryIndex` and a schema fingerprint;
- registers a lazy graph-derived lexical index that is constructed only when
  an opted-in text ranker requests it.

Read commands can force stored direction when compatibility documents require
it.

## Typed code-query storage selection

The typed `search`, `callers`, `callees`, `impact`, `explore`, and `node`
commands open through `compass_query::open`. The default selector prefers a
published persistent store and its matching `store.ref`, which is
checked against store identity, snapshot ID, manifest digest, and graph digest
before query state is created. `--engine json` also uses the compatible JSON
engine without opening the database, while `--engine store`
requires a readable, referenced store and reports corruption or absence
explicitly. The query index remains disposable and keyed by canonical snapshot
bytes for the JSON engine. Its current `compass-code-index/1` FTS tokenizer
uses the shared `compass.search-term/1` analyzer, which applies Unicode case
normalization and combining-mark removal while preserving underscore tokens.
Store queries use immutable snapshot indexes instead of this disposable cache.
The corresponding immutable term-posting layout is
`compass.store.graph-index/1`. Unknown layouts are rejected rather than being
reinterpreted with the current analyzer.

Search candidate truncation is observable and therefore backend-neutral. Both
engines select matching candidates in canonical node-ID order, apply the bound,
then run common Rust ranking and tie-breaking. Store prefix scans traverse the
complete bounded posting range before retaining the smallest candidate IDs;
the posting-chunk count cannot consume a node-candidate limit. Differential
tests include more matching vocabulary entries than the public candidate bound.
`maxCandidates` is the total multi-source recall-pool bound; it is never
inflated to `maxNodes`. The default therefore ranks at most 20 recalled
candidates, while `maxNodes` independently limits how many ranked nodes can be
returned. The 100,000-node in-process ceiling exercises direct search and the
natural-query planner in addition to callers, impact, and node trails.

Each opened code-query engine keeps a 64-entry LRU cache of validated search
preparations. The cache stores only bounded query terms and the derived FTS
expression, is scoped to one graph engine, and is discarded when that engine
closes, so it cannot outlive or cross graph realizations. Ranking retains the
same total deterministic comparator but uses partial top-k selection before
sorting when `maxNodes` is smaller than the candidate set. This avoids sorting
results that the response budget will discard without changing the returned
prefix or truncation semantics.

Bounded typo variants are generated only when the higher-confidence recall
channels produce too few candidates. Diacritic normalization is attempted when
it changes the term, then adjacent transpositions are tried before deletions so
the fixed three-variant budget prioritizes common spelling mistakes. Fuzzy
candidates retain their explicit source penalty in `query-ranker/1`.
`query-ranker/1` is the unconditional typed-search ranker. Runtime profile
selection has been removed, so equivalent inputs do
not depend on process environment. The ranker combines lexical coverage,
candidate provenance, evidence confidence, semantic node kind, and bounded
ambiguity penalties. Production source receives a deterministic advantage over
otherwise equal generated/test declarations. A frozen v1 implementation exists
only in unit tests to prove the reviewed production-versus-generated case; it
is not a runtime fallback.

Natural behavior ranking treats source-backed functions, methods, and
constructors as operation roots alongside role-shaped types. A compact
operation-role channel may finish early only when its predicate and subject
evidence proves that its first candidate dominates omitted declarations and
covers the query's subject terms. A subject-matching role such as `Builder`
cannot finish discovery before a more specific method has been read. Compact
type-declaration exits use the same subject-coverage rule.
Predicate-only methods can use the terminal owner as their subject (for
example, `ClaudeGenerator::Generate`), while a method that already names a
subject, such as `SaveStep`, is not burdened with unrelated receiver tokens.
Matching owner terms still provide a secondary rank signal for nested APIs
such as Java builders and test environments.
Test/generated penalties come from path and exact namespace components, not
CamelCase words inside a production type name: `DicerTestEnvironment` under a
production source root is not treated as a test namespace. Symbol signatures
also contribute bounded direct-field evidence, which lets a parameter concept
distinguish otherwise identical overloads such as `contains(SliceKey)` and
`contains(Slice)`.
The stopword policy retains `all` and `why` because they can distinguish code
identities such as `DetectAll` and `runAttributionWhy`. Paths under `e2e/` are
ranked as test support, like `tests/` and language-specific test filenames, so
an otherwise equal production declaration remains first.

## Natural-language intent routing

`compass ask "<question>"` uses the deterministic `query-planner/1` profile
before executing the existing typed query operations. `compass query` also
selects this path for a high-confidence question against a current typed graph.
High-confidence forms route callers, callees, impact, and source-to-target path
questions to their matching `compass.query/1` operation. Search scaffolding
such as `where is` or `find` is removed before symbol search, which avoids
spending recall terms on question prose. Explicit `search`, `callers`,
`callees`, `impact`, and `node` commands retain their existing behavior.

The planner is deliberately bounded and conservative. Questions above 4,096
bytes fail before graph work. Empty, low-confidence, or contradictory requests
fall back to bounded search; ambiguous symbol resolution remains an explicit
`ambiguous_match` diagnostic and never selects a convenient candidate. Planner
rules are local, credential-free, deterministic, and share the request limits,
heuristic-evidence gate, backend behavior, and response envelope of the typed
operation they select. Generic or contradictory questions, historical `--at`
queries, and requests carrying `--traverse` or a text-traversal control remain
on the established relevance traversal. MCP `query_graph` uses the same routing
rule for typed graphs unless a legacy `mode`, `depth`, `token_budget`, or
`context_filter` field is present.

Structural operand resolution uses the same bounded exact-ID, normalized-name,
alias, term-posting, and typo recall assembly as search. Duplicate exact names
remain ambiguous. For a non-exact operand, a candidate with unique evidence in
the operation's required relationship role can resolve the operand; otherwise
the engine returns `ambiguous_match` rather than selecting the top-ranked
candidate. Relation probes are bounded by the request's candidate limit and a
one-edge existence check per candidate.

Node trails traverse published edges from source to target. When no directed
path is found, one undirected probe using the remaining traversal budget
distinguishes a true no-match from
a route that requires traversing at least one edge backward. The latter returns
`direction_mismatch` and publishes no
path, preventing consumers from treating an inverted relationship as valid.

## Relevance qualification

`crates/compass-query/tests/relevance_qualification.rs` keeps three evidence
layers separate: an 80-question metric-contract fixture, a 500-question
AI-reviewed synthetic executable matrix, and a 23-question production-shaped
cross-backend subset. The 500 records cover every query class and send each
natural-language question through `query_natural_profiled` on the digest-pinned
support graph. They require exact rank-one and recall, intent/slot, directed
edge/path, no-answer, and bounded-work thresholds. Their review notes explicitly
state that they are synthetic and not production telemetry. The 23-question
subset maps actual
`CodeQueryResponse` values into ordered node IDs, directed edges, paths,
truncation/no-answer state, measured latency, and serialized response bytes.
JSON/store parity and repeated store execution must agree once timing is
normalized away.

Run `python3 scripts/qualify_query_relevance.py` with an external
`CARGO_TARGET_DIR`. The native gate evaluates Success@1, MRR, Recall@k, nDCG,
intent macro-F1, edge direction, path acceptance, no-answer precision,
backend parity, deterministic ordering, and latency percentiles. The separate
`compass.query-execution-profile/1` envelope records intent, recall, ranking,
execution, and total timing plus candidates read, postings decoded, nodes and
edges expanded, and serialized response bytes. Ordinary `compass.query/1`
responses contain no timing fields and remain byte-deterministic.
The reviewed executable threshold block lives beside the test so a failure is
local and deterministic. Updating its expected identities, graph digest, or
minimums requires an intentional review rather than copying a new result.
The generated 500-record artifact is checked against
`scripts/generate_query_relevance_corpus.py --check` before execution. Fuzzy
recall covers bounded adjacent transposition, deletion, insertion, and
substitution variants (192 per eligible term and 256 per query), and a
512-entry per-engine LRU caches immutable name-lookup results. The cache is
keyed by normalized variant and result limit, so it cannot reuse an unbounded
or differently truncated result.
`scripts/prepare_query_relevance_review.py` turns approved local JSONL query
logs into bounded, redacted, deterministic review candidates. It never sends
data, invents expected results, or writes directly to the judgment corpus; a
two-person review must bind every accepted question to an immutable graph
revision and explicit expected identities. This supplies a safe feedback loop
for unseen paraphrases and domain vocabulary without adding a runtime model,
embedding service, or self-modifying ranker.

## Focused discovery path

```text
question
  -> plan_natural_query()
     -> high-confidence + current typed graph
        -> search/callers/callees/impact/node-trail
        -> compass.query/1
     -> generic, historical, contradictory, or traversal-controlled
        -> query_terms()
        -> score_nodes() [default full-scan profile]
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

Natural-language terms use a bounded, deterministic morphology pass for common
code-query forms (`routes` -> `route`, `registered` -> `register`, and
`routing` -> `route`). Labels and questions use the same canonical form, so
camel-case identifiers can contribute their component tokens without adding a
model or a synonym database. This is still lexical retrieval, not embedding or
semantic inference; a conceptual question can remain under-specified and must
not be treated as an authoritative answer without an independently reviewed
golden judgment.

### Scoring

`score_nodes` retains the established `text-ranker/full-scan-v1` behavior: it
prepares label evidence and scans the loaded graph to rank candidates.
`find_node` and `pick_scored_endpoint` support commands that need one entity.
When at least two query terms are exact components of one compound identifier,
that co-occurrence receives a bounded ranking boost. Per-query label
normalization and tokenization are prepared once before scoring, avoiding a
second full normalization pass while preserving deterministic tie-breaking.

Ranking changes can alter which subgraph users see even when graph data is
unchanged. Protect them with realistic query fixtures and stable tie behavior.

#### Shadow BM25 profile

`score_nodes_with_profile` and `query_graph_text_page_with_profile` expose the
opt-in `text-ranker/bm25-v1` library profile. The CLI and MCP defaults do not
select it. The profile lazily derives an in-memory index from the validated
graph's stable ID, display label, qualified name, kind, and source path fields.
`graph.json` remains authoritative; the index is disposable, local, and reused
only for the lifetime of the loaded graph.

The index records deterministic postings, per-field term frequencies, document
lengths, corpus size, and average document length. Candidate retrieval uses
field weights 4.0 for labels, 3.0 for identifiers, 1.0 for kinds, and 0.5 for
source paths with BM25 `k1=1.2` and `b=0.75`. Retrieval is capped at 512 nodes,
retains the best candidate for each query term, and reports truncation. The
existing exact-name tiers, source-backed/callable preference, generated/test
penalty, stable ID tie-break, seed selection, traversal, provenance rendering,
and pagination then operate over those candidates.

This is qualification code, not a default cutover. Typed JSON FTS5 and store
postings are unchanged, and FTS5's backend-specific `bm25()` score is not used.
Promotion requires a separately reviewed relevance, cold-start, warm-latency,
and memory decision.

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
