# CompassQL 1

CompassQL is Compass's deterministic, read-only structural query language. It is a documented subset of openCypher that executes directly over an immutable Compass graph snapshot. It does not invoke a model, access a network, mutate the graph, or copy data into another database.

## Commands

Natural-language discovery and structural queries are explicit modes:

```bash
compass query "where is authentication enforced?"
compass query --cql 'MATCH (f:Function)-[:CALLS]->(a) RETURN f.id, a.id'
compass query --cql --file queries/auth.cypher --param target=authorize
compass query --cql --file query.cypher --params-file params.json
compass query --cql --stdin --format json
compass query --cql --repl
```

Compass never guesses the mode from query text. The `graphify` compatibility executable does not expose `--cql` because the frozen Python oracle has no equivalent flag.

Exactly one query source is required: one positional argument, `--file`, `--stdin`, or `--repl`. Files and stdin are limited to 1 MiB. A parameter file must be a JSON object no larger than 16 MiB. `--param name=value` parses JSON scalars/lists/maps and otherwise uses a string.

Output formats are `table`, `json`, and `jsonl`. `--output PATH` writes the completed rendering atomically. A limit, timeout, cancellation, or execution error produces no successful partial result.

JSON uses the version tag `compass.cql.result/1`, explicit typed values, columns, rows, and optional plan/profile objects. JSONL uses `compass.cql.jsonl/1`: one header, one object per row, then one summary. These tags are compatibility boundaries; consumers should reject unknown major versions.

## Graph mapping

- Each Compass node is a Cypher node. Stable `id` and display `label` are always properties; stored attributes retain their names.
- The single Cypher label is derived from `file_type`; a missing or unusable identifier falls back to `:Entity`.
- Each stored edge is a directed relationship. Its type is the normalized uppercase `relation`; missing values become `RELATES_TO`.
- Relationship attributes retain their names. Missing `confidence` reads as `EXTRACTED`.
- Parallel relationships stay distinct.

Use `n.id` for the portable stable Compass string ID. `id(n)` and `id(r)` return snapshot-local integer indexes and must not be persisted or compared across graph snapshots.

## Supported language

CompassQL 1 supports `MATCH`, multiple patterns and repeated-variable joins, `OPTIONAL MATCH`, `WHERE`, correlated one-level `EXISTS { MATCH ... }` and openCypher's `EXISTS { (...)-->() }` shorthand, `UNWIND`, `WITH`, `RETURN`, projection wildcards, `DISTINCT`, `UNION`, `UNION ALL`, `ORDER BY`, `SKIP`, and `LIMIT`.

Expressions include scalar/list/map literals, parameters, property and label access, boolean and comparison operators, three-valued null logic, `IN`, safe regex/string predicates, arithmetic, simple/searched `CASE`, and list indexing/slicing.

Functions include:

- Graph: `id`, `labels`, `type`, `nodes`, `relationships`, `length`, `properties`, `keys`.
- Lists: `any`, `all`, `none`, `single`, `size`, `head`, `last`.
- Conditional/string: `coalesce`, `toLower`, `toUpper`, `trim`, `split`, `replace`.
- Conversion: `toInteger`, `toFloat`, `toString`, `toBoolean`.
- Aggregation: `count`, `min`, `max`, `sum`, `avg`, `collect`, including accepted `DISTINCT` forms.

Fixed, bounded variable-length, `shortestPath`, and `allShortestPaths` patterns are supported. Every variable-length relationship needs an explicit upper bound, and no bound may exceed 32. A relationship cannot repeat within one matched path.

```cypher
MATCH p=shortestPath(
  (endpoint:Function)-[:CALLS|IMPORTS_FROM*1..8]->(authorization:Function)
)
WHERE authorization.label = $target
  AND all(edge IN relationships(p) WHERE edge.confidence = 'EXTRACTED')
RETURN endpoint.id, p, length(p) AS hops
ORDER BY hops
LIMIT 100
```

## Planning and profiling

Prefix a query with `EXPLAIN` to compile and show its operator/optimization plan without execution. `PROFILE` executes and returns per-clause input/output rows, candidate nodes, expanded relationships, peak working-memory estimates, elapsed time, and cancellation checkpoints.

Compiler cache keys include exact source, CompassQL/planner versions, ordered parameter types, planning limits, and graph schema fingerprint. Cached plans contain no graph values, parameters, deadlines, cancellation state, or result rows.

## Limits

Interactive defaults are:

| Limit | Default |
| --- | ---: |
| Deadline | 5 seconds |
| Returned rows | 10,000 |
| Path depth | 32 |
| Expanded relationships | 5,000,000 |
| Working memory | 256 MiB |

The corresponding flags are `--timeout-ms`, `--max-rows`, `--max-path-depth`, `--max-expanded-relationships`, and `--max-memory-bytes`. Values may lower but never raise the language path ceiling.

## Diagnostics and exits

Diagnostics carry stable codes and byte spans. The main families are:

| Family | Meaning |
| --- | --- |
| `CQL1001`–`CQL1027` | source, syntax, unsupported read-only surface, or literal errors |
| `CQL2002`–`CQL2020` | scope, type, function, projection, UNION, or path-shape errors |
| `CQL3000`–`CQL3008` | source/token/nesting/path/row/expansion/memory/time/cancellation limits |
| `CQL4001`–`CQL4099` | parameter, runtime type, regex, arithmetic, value-range, or internal invariant errors |

CLI exit 2 means source/options/compile failure, exit 3 means graph loading failure, and exit 4 means execution, limit, cancellation, or output failure. No diagnostic includes parameter values or credentials.

## Portability and unsupported syntax

CompassQL is not full openCypher, Neo4j Cypher, or ISO GQL. Mutation (`CREATE`, `MERGE`, `DELETE`, `SET`, `REMOVE`, `FOREACH`), procedures/`CALL`, `LOAD CSV`, schema/administration commands, dynamic execution, arbitrary nested subqueries, user-defined functions, and unbounded paths are rejected with stable `CQL1xxx`–`CQL4xxx` diagnostics. Unsupported constructs are never approximated.

See [COMPASSQL_SUPPORT.md](COMPASSQL_SUPPORT.md) for the checked feature matrix.
