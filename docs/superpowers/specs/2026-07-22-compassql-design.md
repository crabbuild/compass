# CompassQL Core Design

**Status:** Approved

**Date:** 2026-07-22

**Product:** Compass

**Primary commands:** `compass cql`, `compass check`

## Summary

CompassQL is Compass's deterministic structural query language and architecture-policy foundation. It is a documented, read-only subset of openCypher implemented natively over Compass graph snapshots. It complements, but does not replace, the existing natural-language `compass query` command.

CompassQL is a core product interface. It supports property-graph pattern matching, bounded path traversal, joins, optional matches, correlated existence tests, aggregation, parameterized input, result composition, plan inspection, and architecture enforcement. It never invokes a model and never mutates a graph.

Architecture policies remain valid read-only Cypher. A policy passes when its query returns zero rows. Each returned row is one violation. Compass stores policy metadata separately from the portable `.cypher` source.

## Goals

- Provide deterministic, local structural queries over current and historical Compass graphs.
- Make common code-architecture questions concise using Cypher's visual graph-pattern syntax.
- Enforce architecture rules in local workflows and CI with exact witness evidence.
- Preserve node IDs, relationship direction, relation type, source locations, and `EXTRACTED`, `INFERRED`, or `AMBIGUOUS` confidence in every result.
- Execute directly against Compass graph snapshots without copying the graph into another database.
- Remain fully native Rust with no Python runtime, database server, JVM, or separately installed native library.
- Provide explicit resource limits, cancellation, deterministic results, and stable diagnostics.
- Validate accepted syntax and semantics with the applicable openCypher Technology Compatibility Kit scenarios and Neo4j differential tests.
- Keep policy query files portable to Neo4j and, where its dialect supports the query, FalkorDB.

## Non-goals

- Full openCypher, Neo4j Cypher, or ISO GQL compatibility.
- Graph mutation, schema administration, procedures, or dynamic query execution.
- An embedded secondary graph database.
- Replacing `compass query` natural-language discovery.
- Unbounded path enumeration.
- Arbitrary user-defined functions.
- Hidden approximation of unsupported syntax.
- Source-code suppression comments in the first policy schema.

## Product surface

Compass keeps heuristic discovery and deterministic queries visibly separate:

```bash
# Natural-language discovery
compass query "where is authentication enforced?"

# Deterministic CompassQL
compass cql 'MATCH (f:Function)-[:CALLS]->(a) RETURN f, a'
compass cql --file queries/auth-callers.cypher
compass cql --file queries/auth-callers.cypher --param target=authorize
compass cql --file query.cypher --params-file params.json
compass cql repl

# Architecture enforcement
compass check
compass check .compass/policies/security/
compass check --format sarif --output compass.sarif
compass check --fail-on warning
```

The public language name is **CompassQL**. The command is **`compass cql`**. Documentation describes the language as a "documented, read-only openCypher subset" and never claims complete openCypher compatibility.

`compass cql` and `compass check` remain hidden until their respective compatibility, safety, and performance gates pass, following Compass's completed-command policy.

## Compatibility and versioning

CompassQL has a major language version independent of the Compass binary version. The initial language version is CompassQL 1.

- Backward-compatible additions may enter CompassQL 1 after their conformance tests and support-matrix entries land.
- Removing syntax, changing accepted query semantics, or changing result value types requires a new major language version.
- Policies declare `schema = 1`; their query executes under CompassQL 1 unless a later policy schema explicitly selects another language version.
- Diagnostics state whether rejected syntax is invalid openCypher or valid openCypher outside the supported CompassQL version.
- Every supported clause, expression, function, path form, coercion, null behavior, and limit behavior appears in a checked-in support matrix.

Accepted syntax follows openCypher semantics. Compass-specific behavior is limited to documented graph mapping, deterministic result ordering, bounded execution, snapshot selection, and policy evaluation conventions.

## Graph mapping

Compass graph records map to the Cypher property-graph model without modifying the stored graph.

### Nodes

- A Compass node becomes a Cypher node.
- Its stable Compass `id` and display `label` are always exposed as properties.
- Its remaining stored attributes are exposed as properties using their existing names.
- Its Cypher label derives from `file_type` using the same normalization as `compass export neo4j`.
- A missing or invalid `file_type` maps to `:Entity`, matching the export fallback.
- Multiple labels are not synthesized in CompassQL 1.

Example logical view:

```cypher
(node:Function {
  id: "rust:src/auth.rs:authorize",
  label: "authorize()",
  source_file: "src/auth.rs",
  source_location: "L42"
})
```

### Relationships

- A Compass edge becomes a directed Cypher relationship.
- Its type derives from `relation` using the same uppercase identifier normalization as the Neo4j exporter.
- Stored edge attributes become relationship properties.
- Missing confidence is exposed as `EXTRACTED`, matching existing Compass rendering and export behavior.
- Parallel Compass relationships remain distinct relationships.

Example logical view:

```cypher
-[edge:CALLS {
  confidence: "EXTRACTED",
  context: "call"
}]->
```

### Property values

CompassQL values are `Null`, `Boolean`, signed 64-bit `Integer`, finite 64-bit `Float`, UTF-8 `String`, `List`, `Map`, `Node`, `Relationship`, and `Path`.

- A missing property evaluates to `Null`.
- JSON integers that fit in `i64` become `Integer`.
- JSON decimal values become finite `Float`.
- An integer outside the signed 64-bit range produces a typed value-range diagnostic when a query reads it; loading the graph itself remains compatible.
- Lists and maps are recursively converted and charged to the query memory budget.
- CompassQL uses openCypher three-valued boolean logic for `Null`.
- `n.id` is the portable stable Compass ID. The openCypher `id(n)` function, when used, returns a snapshot-local integer and must not be used for policy identity or cross-database result comparison.

Keywords are case-insensitive. Labels, relationship types, variable names, parameter names, and property names retain openCypher case sensitivity.

## CompassQL 1 language surface

### Clauses

CompassQL 1 supports:

- `MATCH`, including multiple patterns and repeated-variable joins.
- `OPTIONAL MATCH`.
- `WHERE`.
- Correlated `EXISTS { MATCH ... }` and `NOT EXISTS { MATCH ... }` subqueries with one subquery level.
- `UNWIND` for literal and parameter-provided lists.
- `WITH` for projection, aggregation, `DISTINCT`, ordering, and filtering.
- `RETURN` and `RETURN DISTINCT`.
- `UNION` and `UNION ALL` with compatible column names and value types.
- `ORDER BY`, `SKIP`, and `LIMIT`.
- Query prefixes `EXPLAIN` and `PROFILE`.

Aliases use `AS`.

Example:

```cypher
MATCH (caller:Function)-[call:CALLS]->(target:Function)
WHERE target.label = $target
  AND call.confidence = "EXTRACTED"
RETURN caller.id, caller.label, caller.source_file
ORDER BY caller.source_file
LIMIT 100
```

### Expressions

CompassQL 1 supports:

- Equality and ordering: `=`, `<>`, `<`, `<=`, `>`, `>=`.
- Boolean logic: `AND`, `OR`, `NOT`.
- Null predicates: `IS NULL`, `IS NOT NULL`.
- Membership: `IN`.
- String predicates: `STARTS WITH`, `ENDS WITH`, `CONTAINS`, and `=~`.
- Scalar, list, and map literals.
- Property access and label predicates.
- Parenthesized expressions.
- Parameters such as `$target`.
- Simple and searched `CASE`.
- List indexing and slicing where the openCypher semantics are unambiguous.

Regex execution uses Rust's bounded non-backtracking regex implementation. CompassQL documents the safe openCypher-compatible regex subset; valid openCypher regex features outside that subset are rejected explicitly rather than reinterpreted. Regex compilation and working memory count against query budgets.

### Functions

CompassQL 1 includes:

- Graph: `id`, `labels`, `type`, `nodes`, `relationships`, `length`.
- List predicates: `any`, `all`, `none`, `single`.
- List helpers: `size`, `head`, `last`.
- Null and conditional: `coalesce`.
- String: `toLower`, `toUpper`, `trim`, `split`, `replace`.
- Aggregation: `count`, `count(DISTINCT ...)`, `min`, `max`, `sum`, `avg`, `collect`, and `collect(DISTINCT ...)`.

Function arity, null propagation, numeric coercion, and empty-input behavior follow the selected openCypher TCK scenarios.

### Pattern matching and joins

CompassQL supports node labels, relationship types, property maps, directions, variables, and repeated variables.

```cypher
MATCH (handler:Function)-[:CALLS]->(service:Function),
      (test:Function)-[:TESTS]->(handler)
WHERE service.source_file STARTS WITH "src/security/"
RETURN handler, service, test
```

Repeated variables are equijoins on graph identity. Independent patterns are planned in a deterministic selectivity order but preserve openCypher result multiplicity.

### Existence tests

CompassQL supports correlated, read-only existence subqueries containing `MATCH` and `WHERE`. They may reference outer variables but may not introduce mutation, procedure calls, nested subqueries, aggregation, or result-producing clauses.

```cypher
MATCH (endpoint:Function)
WHERE endpoint.source_file STARTS WITH "src/api/"
  AND NOT EXISTS {
    MATCH (endpoint)-[:CALLS*1..4]->(:Function {label: "authorize()"})
  }
RETURN endpoint
```

The planner short-circuits an existence subquery after its first match.

### Optional matching

Variables introduced only by `OPTIONAL MATCH` are nullable. The semantic analyzer tracks nullability, and property access on a null node or relationship evaluates to `Null` as required by openCypher. Queries may filter nullable variables with `IS NOT NULL` or replace null values with `coalesce`.

```cypher
MATCH (function:Function)
WHERE function.source_file STARTS WITH "src/security/"
OPTIONAL MATCH (test:Function)-[:TESTS]->(function)
WITH function, test
WHERE test IS NULL
RETURN function
```

### Aggregation and `WITH`

`WITH` establishes a new variable scope. Non-aggregated projected expressions form grouping keys when aggregates are present.

```cypher
MATCH (module:File)-[:IMPORTS_FROM]->(dependency:File)
WITH module, count(DISTINCT dependency) AS dependencies
WHERE dependencies > 12
RETURN module.source_file, dependencies
ORDER BY dependencies DESC
```

Aggregation state is charged to the query memory budget. Compass aborts rather than producing a partial aggregate.

### `UNWIND`

`UNWIND` supports list literals, parameters, and previously bound list expressions.

```cypher
UNWIND $changed_files AS changed
MATCH (node {source_file: changed})<-[:CALLS|IMPORTS_FROM*1..5]-(affected)
RETURN DISTINCT affected
```

### Set composition

`UNION` removes duplicate rows; `UNION ALL` preserves them. Branches must expose the same column names and compatible value types. Each branch is planned independently under the same snapshot and query budget.

### Bounded paths

CompassQL supports fixed-length paths, bounded variable-length relationships, `shortestPath`, and `allShortestPaths`.

```cypher
MATCH (a)-[:CALLS]->(b)
```

```cypher
MATCH (a)-[:CALLS|IMPORTS_FROM*1..6]->(b)
```

```cypher
MATCH p = shortestPath(
  (a)-[:CALLS|IMPORTS_FROM*1..8]->(b)
)
RETURN p
```

Rules:

- Every variable-length relationship requires an explicit upper bound.
- The CompassQL 1 compiled ceiling is 32 relationships per path.
- Runtime and policy configuration may lower but never raise that ceiling.
- A relationship cannot repeat within one matched path.
- `shortestPath` uses bidirectional BFS when both endpoints are anchored.
- `allShortestPaths` is limited by the result-row, expanded-relationship, memory, and time budgets.
- Path predicates may inspect `nodes(path)` and `relationships(path)` using list predicates.

### Result ordering

`ORDER BY` follows openCypher ordering and null-placement semantics for the accepted value types. Without `ORDER BY`, Compass canonicalizes the bounded final result set by projected value type and value so identical graph snapshots and parameters produce deterministic output. Policy violations are additionally ordered by policy ID and stable violation fingerprint.

### Explicitly unsupported syntax

CompassQL 1 rejects:

- `CREATE`, `MERGE`, `DELETE`, and `DETACH DELETE`.
- `SET`, `REMOVE`, and `FOREACH`.
- Database, graph, user, role, transaction, index, or constraint administration.
- `CALL`, procedures, and user-defined functions.
- `LOAD CSV` and external-resource access.
- Arbitrary nested subqueries.
- Dynamic query execution.
- Unbounded variable-length paths.
- Write-capable or schema-capable clauses introduced by later Cypher or GQL versions.

Recognized but unsupported syntax returns a stable unsupported-feature diagnostic; it is never approximated.

```text
CQL1007: CALL is valid Cypher but unsupported by CompassQL 1
  --> query.cypher:4:1
help: CompassQL does not execute procedures or external resources
```

## Architecture

### Crate responsibilities

CompassQL introduces focused crates and extends existing ones:

- `compass-cypher`: lexer, parser, Compass-owned typed AST, semantic analysis, logical plans, optimizer, and language diagnostics.
- `compass-query`: physical operators and execution over `compass_model::Graph` snapshots.
- `compass-policy`: policy discovery, metadata, validation, exemptions, baselines, evaluation, and policy result types.
- `compass-model`: immutable query indexes and snapshot schema fingerprints.
- `compass-output`: table, JSON, JSONL, SARIF, and policy witness rendering.
- `compass-cli`: `compass cql` and `compass check` argument parsing and dispatch.
- `compass-mcp`: structured `query_cql` and `check_architecture` tools.

No query crate depends on CLI or output presentation. `compass-policy` consumes the CompassQL execution interface and returns structured results without rendering them.

### Data flow

```text
Cypher source
  -> lex and parse into a span-annotated Compass AST
  -> reject unsupported or mutating syntax
  -> bind variables and infer value/nullability types
  -> compile a bounded logical plan
  -> optimize without changing result semantics
  -> select one immutable graph snapshot
  -> execute bounded physical operators
  -> canonicalize and stream typed result rows
  -> render table, JSON, JSONL, SARIF, or MCP content
```

Policies use the same path:

```text
policy.toml + portable query.cypher
  -> validate confined paths, metadata, and limits
  -> compile and execute under the policy budget
  -> zero rows: pass
  -> returned rows: violations
  -> apply explicit exemptions or baseline fingerprints
  -> render evidence and determine exit status
```

### Parser ownership

CompassQL 1 uses a Compass-owned lexer and parser generated or implemented from the published openCypher grammar for the supported surface. The parser produces only Compass-owned AST types with byte spans. Runtime behavior does not depend on an alpha third-party Cypher parser.

An internal parser interface permits differential parser tests against open-source parsers during development, but it is not a public extension point:

```rust
pub(crate) trait CypherParser {
    fn parse(&self, source: &str) -> Result<QueryAst, ParseDiagnostics>;
}
```

The expression parser uses explicit precedence matching openCypher. Parser recursion, token count, source length, literal size, and nesting depth are bounded before semantic analysis.

### Semantic analysis

Semantic analysis performs:

- Clause and variable-scope validation.
- Label, relationship-type, and property-name resolution.
- Parameter declaration and runtime type checking.
- Expression type and nullable-type inference.
- Aggregate-context validation.
- `WITH`, subquery, and `UNION` scope validation.
- Function lookup and arity checking.
- Read-only and supported-subset enforcement.
- Path-bound and resource-ceiling validation.

All diagnostics carry stable codes, source spans, and actionable help where one unambiguous rewrite exists.

## Planning and optimization

### Logical operators

Queries compile into a typed logical plan composed from:

```text
NodeScan
RelationshipExpand
VariableExpand
ShortestPath
Filter
Project
Unwind
Optional
Exists
Join
Aggregate
Distinct
Sort
Skip
Limit
Union
```

Each operator declares input and output columns, value and nullability types, estimated cardinality, working-memory estimate, required graph indexes, deterministic-order behavior, and cancellation checkpoints.

### Initial optimizer rules

The optimizer uses deterministic rules rather than an opaque cost-based system:

- Anchor exact node IDs before indexed labels or properties, and indexed properties before scans.
- Push node and relationship predicates into scans and expansions.
- Restrict adjacency iteration by relationship type and direction.
- Reorder independent patterns from the most selective stable anchor.
- Short-circuit `EXISTS` after its first match.
- Push `LIMIT` through operators only when openCypher semantics permit.
- Remove unused projections and properties.
- Select hash or index joins only when their memory upper bound fits the query budget.
- Use bidirectional BFS for anchored shortest paths.
- Refuse unbounded or statically impossible-to-budget plans before execution.

An optimized plan and its unoptimized reference plan must return equivalent normalized results in property tests.

### `EXPLAIN` and `PROFILE`

`EXPLAIN` parses, validates, and plans without executing. It reports operators, variables, indexes, estimates, path bounds, and warnings.

`PROFILE` executes and reports actual input rows, output rows, candidate nodes, expanded relationships, peak operator memory, elapsed time, cancellation checks, and cache use. Profiling data never changes query results.

## Graph indexes

The existing node-ID and incoming/outgoing adjacency indexes remain foundational. Immutable graph snapshots add:

- Node label index.
- Relationship type index.
- Exact display-label index.
- Exact `source_file` index.
- Relationship-type-specific outgoing and incoming adjacency.
- Configured exact-property indexes for explicitly selected high-cardinality properties.

Arbitrary properties support bounded scans. A query never silently creates or persists an index. Snapshot indexes are built once and shared across policy queries.

## Execution model

Execution uses bounded batches. Most operators do not materialize their complete input. `Sort`, aggregation, `DISTINCT`, some joins, `OPTIONAL MATCH`, and deterministic final canonicalization may buffer rows and must reserve memory from the query budget before growing.

```rust
pub enum CompassValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(std::sync::Arc<str>),
    List(std::sync::Arc<[CompassValue]>),
    Map(std::sync::Arc<PropertyMap>),
    Node(NodeRef),
    Relationship(RelationshipRef),
    Path(PathRef),
}
```

`NodeRef`, `RelationshipRef`, and `PathRef` refer into the immutable snapshot and remain valid for the execution lifetime. Complete graph records are not copied into intermediate rows.

CompassQL 1 aborts on memory exhaustion rather than spilling sensitive graph data. A future spill design requires a separate approved specification with repository-confined temporary files, encryption and cleanup behavior, and cross-platform tests.

## Resource limits and cancellation

Default limits are:

| Resource | Interactive `compass cql` | One policy query |
|---|---:|---:|
| Execution time | 5 seconds | 2 seconds |
| Returned rows | 10,000 | 1,000 violations |
| Path depth | 32 | Metadata value, maximum 32 |
| Expanded relationships | 5,000,000 | 2,000,000 |
| Working memory | 256 MiB | 128 MiB |
| Query source | 1 MiB | 1 MiB |

CLI options may lower these values. Raising them is allowed only up to documented compiled ceilings and is always shown in `EXPLAIN`, `PROFILE`, and diagnostics.

Cancellation is checked during scans, expansions, joins, existence tests, aggregation, sorting, canonicalization, and path search. A timeout, cancellation, memory failure, row limit, or expansion limit never becomes a partial policy pass.

## Parameters

`--param name=value` supplies one UTF-8 string parameter. `--params-file path.json` supplies typed parameters from one JSON object. Duplicate names, undeclared invalid names, non-object parameter files, and values outside CompassQL's type model are errors.

Parameters are values and are never interpolated into query source. Parameter files are size-capped and read through Compass's bounded file utilities.

The MCP interface accepts typed JSON parameters directly.

## Plan caching

The resident daemon may cache parsed ASTs and logical plans by:

```text
query digest
+ parameter type signature
+ CompassQL major version
+ graph schema fingerprint
+ planner version
```

Plans contain no graph values. A plan may execute against a newer immutable snapshot only when its schema fingerprint and planner compatibility key match. Physical operator state and prior result rows are never reused.

## Architecture policies

### Discovery and layout

The default policy root is `.compass/policies/` beneath the discovered Git worktree root. Each policy has one directory:

```text
.compass/
└── policies/
    ├── domain-isolation/
    │   ├── policy.toml
    │   └── query.cypher
    └── authorization/
        ├── policy.toml
        └── query.cypher
```

Discovery is recursive, lexically sorted, and does not follow symbolic links. Metadata and query paths must remain beneath the policy directory after canonicalization.

### Metadata schema

```toml
schema = 1
id = "architecture.domain-isolation"
query = "query.cypher"
severity = "error"
message = "Domain code must not depend on database implementation"
owners = ["platform-architecture"]
tags = ["architecture", "dependency-direction"]

[limits]
timeout_ms = 2000
max_rows = 1000
max_path_depth = 8
max_expanded_relationships = 2000000
memory_mib = 128
```

Required fields are `schema`, `id`, `query`, `severity`, `message`, and at least one owner. Valid severities are `error`, `warning`, and `info`. Unknown metadata fields are rejected in schema 1 so misspellings cannot silently weaken enforcement.

Policy limits may only lower global policy defaults. Query parameters, if needed, are declared as typed TOML values under `[parameters]`; CI flags cannot silently override them.

### Violation contract

- Zero returned rows means the policy passes.
- Every returned row is one violation.
- A returned `Path` aliased as `witness` is the primary explanation.
- Other returned nodes, relationships, paths, and scalar values become structured evidence.
- A query without a `witness` remains valid and renders its complete result row.
- A query or execution error fails evaluation; it never counts as zero rows.

```cypher
MATCH p =
  (domain:File)-[:IMPORTS_FROM|CALLS*1..8]->(database:File)
WHERE domain.source_file STARTS WITH "src/domain/"
  AND database.source_file STARTS WITH "src/database/"
RETURN p AS witness, domain, database
```

### Violation identity

A violation fingerprint is a versioned SHA-256 digest over:

```text
fingerprint schema
+ policy ID
+ returned column names
+ stable Compass IDs for returned nodes and relationships
+ canonical scalar and collection values
+ ordered stable IDs in returned paths
```

Source line numbers, timestamps, display formatting, and transient graph indexes are excluded. Fingerprints are encoded as `cmpv1:<lowercase-hex>`.

### Exemptions

Exemptions live in `policy.toml` and require identity, accountability, and expiration:

```toml
[[exemptions]]
fingerprint = "cmpv1:..."
reason = "Legacy path being removed in migration ARCH-241"
owner = "platform-architecture"
expires = "2026-10-01"
```

- `reason`, `owner`, and ISO-8601 calendar-date `expires` are mandatory.
- Expired exemptions fail the policy check.
- Unknown fingerprints produce warnings so stale exemptions are removed.
- One exemption suppresses only its exact versioned fingerprint.
- Source-code suppression comments are unsupported in policy schema 1.

### Baselines

`compass check --write-baseline PATH --confirm` writes a versioned canonical JSON baseline of current fingerprints. The explicit `--confirm` flag is required in non-interactive and interactive use.

- A baseline suppresses only listed existing fingerprints.
- Newly introduced fingerprints remain violations.
- Missing baseline files are errors.
- Stale baseline entries produce warnings.
- Baseline updates are explicit and deterministic.
- Baselines never suppress query, graph, timeout, cancellation, or limit failures.

## Outputs

`compass cql` supports `table`, `json`, and `jsonl`. The TTY default is `table`; redirected output retains the explicitly selected format rather than guessing a schema.

`compass check` supports human-readable terminal output, canonical JSON, JSONL violations, and SARIF. Every output includes the policy ID, severity, message, owners, policy location, fingerprint, exemption state, and structured evidence.

The default failure threshold is `error`. `--fail-on error|warning|info` may make lower severities fail the run; it cannot disable `error` failures.

Human witness rendering preserves relationship direction, type, confidence, and source location:

```text
error[architecture.domain-isolation]
Domain code must not depend on database implementation

src/domain/order.rs:18 OrderService
  --CALLS [EXTRACTED]-->
src/application/load_order.rs:31 load_order
  --IMPORTS_FROM [EXTRACTED]-->
src/database/postgres.rs PostgresOrderRepository

Owner: platform-architecture
Policy: .compass/policies/domain-isolation/policy.toml
```

## Exit status

```text
0  all applicable policies passed
1  policy violations met the configured failure severity
2  invalid query, policy metadata, parameters, or unsupported syntax
3  graph unavailable, corrupt, or incompatible
4  resource limit, cancellation, timeout, or internal execution failure
```

`compass cql` uses `0` for successful execution and the same `2`, `3`, and `4` categories for failures. An empty successful result is exit `0`.

## Diagnostics

Diagnostics include a stable code, category, source label, byte span rendered as line and column, explanation, and an actionable help message when one correct rewrite exists.

```text
CQL2004: variable `service` is not in scope after WITH
  --> architecture.cypher:8:18
   |
8  | RETURN service.source_file
   |        ^^^^^^^
help: include `service` in the preceding WITH projection
```

Diagnostic families are:

- `CQL1xxx`: lexical, parse, and unsupported-syntax errors.
- `CQL2xxx`: scope, type, nullability, function, and parameter errors.
- `CQL3xxx`: planning and static resource-limit errors.
- `CQL4xxx`: runtime, cancellation, and dynamic resource-limit errors.
- `CPL1xxx`: policy discovery, metadata, exemption, and baseline errors.

## MCP integration

Compass MCP exposes:

- `query_cql`: source, typed parameters, format-independent row limit, optional graph selection, and structured rows.
- `explain_cql`: source and parameter types, returning a structured logical and physical plan without execution.
- `check_architecture`: policy roots or IDs, graph selection, failure severity, and structured policy results.

MCP calls use stricter server-configured budgets when those are lower than query or policy defaults. MCP responses never contain terminal escape sequences or preformatted SARIF.

## Historical and daemon integration

CompassQL consumes a graph-selection interface rather than opening files directly. The initial selector is the current graph or explicit `--graph PATH`. When versioned graph history lands, the same interface adds `--at REV` without changing query semantics.

```bash
compass cql --file query.cypher --at HEAD~20
compass check --at main
```

The resident daemon shares immutable snapshots and indexes across queries and policy suites. It may cache plans under the compatibility key defined above. Publishing a new snapshot is atomic; in-flight queries finish against their original snapshot.

History-aware diff enforcement is a separate command behavior built on the same fingerprints:

```bash
compass check --against main --new-only
```

It reports violations present in the selected current realization but absent from the comparison realization. It never hides an evaluation error on either side.

## Security

- CompassQL execution has no filesystem, network, environment-variable, process, or provider access.
- Query parameters are typed values and never source interpolation.
- Parser recursion, token count, source size, literal size, regex size, collection size, and nesting depth are bounded.
- Regex uses a non-backtracking implementation.
- Operators cannot bypass path, expansion, row, memory, or time budgets.
- Result renderers independently escape table, JSON, JSONL, SARIF, and MCP representations.
- Diagnostics do not dump unrelated properties or credentials.
- Policy discovery and baseline writes reject symbolic-link and canonical-path escapes.
- `EXPLAIN` cannot read graph property values beyond schema and index statistics.
- `PROFILE` reports counts and timings without leaking unrelated row content.
- Mutation clauses are rejected before planning and cannot reach the graph execution interface.

## Conformance and testing

### Support matrix and openCypher TCK

The checked-in support matrix maps every CompassQL feature to applicable openCypher TCK scenarios.

Tests are classified as:

1. Supported TCK scenarios that must pass.
2. Unsupported scenarios that must return the documented diagnostic family.
3. Mutation and administration scenarios that must be rejected before planning.

Expanding CompassQL converts explicit unsupported cases into conformance cases. A release cannot reduce the passing supported set within one CompassQL major version.

### Differential tests

Representative accepted queries run against the native CompassQL engine and the same graph exported to Neo4j. FalkorDB participates where its documented dialect supports the query.

Results normalize by column name, value type, stable exported node ID, stable relationship tuple, path direction, multiplicity, and explicit ordering. Differential fixtures cover:

- Null and three-valued logic.
- Numeric comparison and coercion.
- Duplicate rows and `DISTINCT`.
- Multiple patterns and joins.
- `OPTIONAL MATCH`.
- `EXISTS` and `NOT EXISTS`.
- `UNWIND`.
- Aggregation and `WITH`.
- `UNION` and `UNION ALL`.
- Variable-length and shortest paths.
- Relationship uniqueness and parallel relationships.
- Lists, maps, string functions, regex, and `CASE`.

Neo4j is the primary semantic oracle. FalkorDB differences are documented and do not redefine local CompassQL semantics.

### Test layers

- Lexer and parser golden tests.
- AST span and recovery tests.
- Semantic scope, type, and nullability tests.
- Logical-plan snapshot tests.
- Physical-operator unit tests.
- Optimizer equivalence property tests.
- TCK conformance tests.
- Neo4j and FalkorDB differential tests.
- Policy discovery, exemption, baseline, and witness tests.
- CLI and MCP contract tests.
- Linux, macOS, and Windows tests on x86-64 and ARM64 release targets.
- Fuzzing for source text, parameters, ASTs, plans, and hostile graph properties.
- Mutation testing for policy pass/fail, fingerprinting, and resource guards.
- Miri coverage for safe graph and query operator crates where supported.

## Performance qualification

Release gates measure parse, semantic analysis, planning, and execution separately.

- Cached-plan execution overhead must be no greater than 10% over equivalent direct Compass traversal.
- Anchored one-hop queries must scale with matching adjacency, not total graph size.
- Bounded path queries must stop at the first applicable limit.
- A policy suite loads one immutable snapshot and shares indexes.
- Peak working memory must not exceed the declared query budget.
- Cancellation must complete within 100 ms of the next operator checkpoint.
- CompassQL must not regress existing natural-language query or graph-build benchmarks.

Benchmarks include small, medium, monorepo, high-degree, long-chain, cyclic, parallel-edge, and adversarial graphs. Reports include cold and warm p50, p95, peak RSS, expanded relationships, and result cardinality.

## Delivery gates

Implementation proceeds in independently testable gates:

1. Language frontend, Compass AST, diagnostics, support matrix, and TCK harness.
2. Fixed patterns, filtering, projection, typed parameters, and deterministic results.
3. Bounded variable paths, shortest paths, and path inspection.
4. Joins, `OPTIONAL MATCH`, correlated existence tests, and `UNWIND`.
5. Aggregation, `WITH`, `UNION`, list/string expressions, regex, and `CASE`.
6. `EXPLAIN`, `PROFILE`, optimizer equivalence, plan caching, and performance qualification.
7. Policy metadata, exemptions, baselines, witnesses, SARIF, and `compass check`.
8. MCP integration and graph-selection support for historical realizations.

The command surface remains hidden until the relevant gate is complete. Internal partial implementations are never presented as accepted CompassQL syntax.

## Alternatives rejected

### Embed a property-graph database

Embedding Grafeo, CozoDB, or another database could provide more query features initially, but duplicates Compass storage and indexes, expands the dependency and attack surface, complicates strict graph semantics, and makes latency and memory harder to control. Compass keeps its graph as the sole source of truth.

### Translate Cypher to Trustfall

Trustfall is a capable Rust query engine, but translating Cypher scopes, null behavior, optional matching, aggregation, and paths into a second query abstraction creates two semantic systems to maintain. Direct physical operators provide clearer conformance and resource accounting.

### Full openCypher implementation

Full compatibility includes a large language surface unrelated to local code architecture, including mutation, procedures, administration, and unbounded behavior. Compass instead publishes a valuable read-only core with explicit unsupported diagnostics and measurable conformance.

### Compass-specific `ASSERT` or `FORBID` syntax

Adding policy keywords would make query files non-portable and fork Cypher semantics. Compass uses ordinary read-only queries: zero rows pass, and returned rows are violations. Policy metadata remains separate.

## Acceptance criteria

CompassQL Core is complete when:

- `compass cql` exposes every CompassQL 1 feature listed in this document and rejects every listed unsupported feature explicitly.
- `compass check` discovers schema-1 policies, evaluates zero-row pass semantics, renders witnesses, applies accountable exemptions and baselines, and emits stable exit statuses.
- All selected openCypher TCK scenarios pass and the support matrix contains no undocumented accepted syntax.
- Neo4j differential fixtures match after documented normalization.
- Parser, planner, executor, policy, CLI, output, and MCP safety tests pass across supported release platforms.
- Fuzzing, mutation, Miri, and resource-limit gates pass for the scopes defined above.
- Cold and warm performance gates pass without regressing existing Compass query and build baselines.
- Existing `compass query`, graph export, Neo4j push, and Graphify compatibility behavior remain unchanged.
- Documentation includes a CompassQL reference, support matrix, policy authoring guide, migration examples, and exact limit defaults.
