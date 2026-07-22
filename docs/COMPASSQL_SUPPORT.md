# CompassQL 1 support matrix

This matrix is the public compatibility boundary. “Supported” means native parsing, semantic validation, deterministic execution, and checked evidence. CompassQL does not claim support for syntax absent from this table.

| Area | Surface | Status | Evidence |
| --- | --- | --- | --- |
| Clauses | `MATCH`, repeated variables, multiple patterns | Supported | `compass-query/tests/cql_execution.rs` |
| Clauses | `OPTIONAL MATCH`, `WHERE` | Supported | `compass-query/tests/cql_execution.rs` |
| Clauses | one-level correlated `EXISTS { MATCH ... }` and pattern shorthand | Supported | pinned openCypher existential scenarios |
| Clauses | `UNWIND`, `WITH`, `RETURN`, aliases, projection wildcards | Supported | pinned openCypher UNWIND/WITH scenarios |
| Clauses | `DISTINCT`, `UNION`, `UNION ALL` | Supported | executor conformance suite |
| Clauses | `ORDER BY`, `SKIP`, `LIMIT` | Supported | executor conformance suite |
| Prefixes | `EXPLAIN`, `PROFILE` | Supported | compiler core and CLI tests |
| Patterns | labels, types, maps, directions, joins | Supported | executor core tests |
| Paths | fixed and explicitly bounded variable length | Supported, maximum 32 | path tests and `CQL3002` |
| Paths | `shortestPath`, `allShortestPaths` | Supported and budgeted | path conformance suite |
| Expressions | comparison, boolean, null, `IN` | Supported | expression conformance suite |
| Expressions | `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `=~` | Supported; regex safe subset | expression conformance suite |
| Expressions | arithmetic, literals, property/label access | Supported | expression conformance suite |
| Expressions | `CASE`, list index/slice | Supported | expression conformance suite |
| Graph functions | `id`, `labels`, `type`, `nodes`, `relationships`, `length`, `properties`, `keys` | Supported | function conformance suite |
| List functions | `any`, `all`, `none`, `single`, `size`, `head`, `last` | Supported | three-valued list tests |
| Conversion functions | `toInteger`, `toFloat`, `toString`, `toBoolean` | Supported | openCypher list and scalar conformance tests |
| String/null | `coalesce`, `toLower`, `toUpper`, `trim`, `split`, `replace` | Supported | function conformance suite |
| Aggregates | `count`, `min`, `max`, `sum`, `avg`, `collect`, `DISTINCT` | Supported | aggregate conformance suite |
| Mutation | create/update/delete/schema clauses | Rejected (`CQL1007`) | compiler rejection tests |
| External execution | `CALL`, procedures, `LOAD CSV`, dynamic queries | Rejected (`CQL1007`) | compiler rejection tests |
| Paths | unbounded variable length | Rejected (`CQL3002`) | compiler rejection tests |
| Subqueries | nested/arbitrary result-producing subqueries | Rejected (`CQL1008`/`CQL2019`) | semantic rejection tests |

Accepted behavior follows the pinned read-only openCypher 2024.3 scenarios listed in `tests/opencypher-tck/manifest.toml`. Compass-specific behavior is limited to graph mapping, deterministic unordered results, bounded execution, snapshot selection, and stable diagnostics.
