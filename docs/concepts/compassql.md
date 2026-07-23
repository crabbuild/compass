# CompassQL concepts

CompassQL is Compass's deterministic, read-only structural query language. It
uses a documented subset of openCypher to match exact graph patterns without
copying the graph into another database.

> **Who this page is for:** users deciding between discovery commands and
> CompassQL, plus integrators preparing stable automated queries.
>
> **You will learn:** when CompassQL is appropriate, how Compass data maps into
> the language, how queries flow through the engine, and how limits protect
> automation.
>
> **Prerequisites:** familiarity with [the graph model](graph-model.md). This is
> a concept guide; use [CompassQL 1](../COMPASSQL.md) as the canonical syntax
> and runtime reference.
>
> **Reading time:** 8–10 minutes.

## Discovery and structural matching solve different problems

Use natural-language discovery when you do not yet know the exact graph shape:

```bash
compass query "where is authentication enforced?"
```

Use CompassQL when you can state the pattern:

```bash
compass query --cql \
  "MATCH (caller)-[:CALLS]->(target)
   WHERE target.label = 'authorize'
   RETURN caller.id, target.id
   LIMIT 20"
```

The modes are explicit. Compass never guesses that text happens to be Cypher.

| Need | Best starting point |
| --- | --- |
| Find likely concepts from a phrase | `compass query "..."` |
| Inspect one node | `compass explain ...` |
| Connect two known nodes | `compass path ... ...` |
| Estimate reverse impact | `compass affected ...` |
| Match an exact pattern | `compass query --cql ...` |
| Produce typed JSON or JSONL | CompassQL with `--format` |
| Explain or profile execution | `EXPLAIN` or `PROFILE` CompassQL |

## Data mapping

Each Compass node becomes a Cypher node:

```text
Compass node
  id: "src/auth.rs::verify"
  label: "verify()"
  file_type: "Function"

CompassQL view
  (:Function {
      id: "src/auth.rs::verify",
      label: "verify()",
      file_type: "Function",
      ...
  })
```

The single Cypher label is derived from `file_type`. If that value cannot form
a usable identifier, Compass uses `:Entity`.

Each Compass edge becomes a directed relationship:

```text
relation: "calls"  ->  [:CALLS]
```

Missing relation values map to `RELATES_TO`. Stored edge attributes remain
properties. Parallel relationships remain distinct.

## Stable and snapshot-local identity

Two identity functions look similar but serve different purposes:

```cypher
RETURN n.id, id(n)
```

- `n.id` is the stable Compass string ID. Persist this when an integration must
  refer to the same graph entity.
- `id(n)` is an integer index for one loaded snapshot. It must not be persisted
  or compared across snapshots.

The same rule applies to relationship indexes returned by `id(r)`.

## A query is compiled, planned, bounded, and rendered

```text
query source
    |
    v
lexer and parser
    |
    v
semantic validation and type/scope checks
    |
    v
logical plan and optimizations
    |
    v
bounded execution over graph indexes
    |
    v
table, JSON, JSONL, EXPLAIN, or PROFILE output
```

Unsupported syntax is rejected. CompassQL does not approximate mutations,
procedures, unbounded paths, or arbitrary nested subqueries.

This conservative behavior is important for automation: a query either means
what the documented subset says or produces a diagnostic.

## Parameters

Use parameters when values come from a user, environment, or script:

```bash
compass query --cql \
  'MATCH (n) WHERE n.label = $target RETURN n.id' \
  --param target=authorize
```

`--param name=value` attempts to parse JSON values such as numbers, booleans,
lists, maps, and `null`; otherwise it uses a string. A parameters file must be
a JSON object:

```json
{
  "target": "authorize",
  "relations": ["CALLS", "USES"],
  "maximumHops": 6
}
```

```bash
compass query --cql --file queries/auth.cypher \
  --params-file queries/auth-params.json
```

Parameters separate data from query structure. They also avoid fragile quoting
and make plan-cache type keys explicit. They are not a license to log secrets:
Compass diagnostics avoid parameter values, and your surrounding script should
do the same.

## Bounded paths

CompassQL supports fixed and bounded variable-length patterns:

```cypher
MATCH p=(entry)-[:CALLS|IMPORTS_FROM*1..6]->(target)
WHERE target.label = $target
RETURN entry.id, length(p) AS hops
ORDER BY hops
LIMIT 100
```

Every variable-length relationship requires an explicit upper bound, and the
language ceiling is 32. This avoids accidental unbounded traversal.

Shortest paths are also bounded:

```cypher
MATCH p=shortestPath(
  (entry:Function)-[:CALLS|IMPORTS_FROM*1..8]->(auth:Function)
)
WHERE auth.label = $target
RETURN entry.id, p, length(p) AS hops
```

A relationship cannot repeat in one matched path.

## Null and optional matching

CompassQL follows three-valued null logic. An `OPTIONAL MATCH` can retain an
input row even when the optional pattern does not match:

```cypher
MATCH (service:Class)
OPTIONAL MATCH (service)-[:CALLS]->(dependency)
RETURN service.id, dependency.id
```

The second column can be null. Test for null explicitly and do not assume a
missing relationship means the source entity is unused; it means that pattern
did not match in this snapshot.

## Output formats

### Table

Table output is for people:

```bash
compass query --cql 'MATCH (n) RETURN n.id LIMIT 5'
```

### JSON

JSON is one complete, versioned result document:

```bash
compass query --cql 'MATCH (n) RETURN n.id LIMIT 5' --format json
```

The schema tag is `compass.cql.result/1`.

### JSONL

JSONL emits a versioned header, one object per row, and a summary:

```bash
compass query --cql 'MATCH (n) RETURN n.id LIMIT 5' --format jsonl
```

The stream tag is `compass.cql.jsonl/1`.

Consumers must reject unknown major versions rather than guessing.

`--output PATH` writes the completed rendering atomically. A timeout, limit,
cancellation, or execution failure does not produce a successful partial
result.

## Limits are part of correctness

Interactive defaults bound:

- deadline;
- returned rows;
- path depth;
- expanded relationships;
- working memory.

The corresponding flags are:

```text
--timeout-ms
--max-rows
--max-path-depth
--max-expanded-relationships
--max-memory-bytes
```

Use a smaller limit when a caller has a tighter budget. Do not treat a limit
error as an empty result.

Query files and stdin are also size-limited, and parameter files have a
separate cap. Consult [CompassQL 1](../COMPASSQL.md) for current numeric values.

## Explain and profile

`EXPLAIN` compiles and shows a plan without running the query:

```cypher
EXPLAIN
MATCH (caller)-[:CALLS]->(target)
WHERE target.label = $target
RETURN caller.id
```

Use it to confirm scope, operators, and optimizations.

`PROFILE` executes and adds per-clause measurements such as:

- input and output rows;
- candidate nodes;
- expanded relationships;
- peak working-memory estimate;
- elapsed time;
- cancellation checkpoints.

Profiling is diagnostic evidence from one graph and run. Do not treat a small
fixture's profile as a production capacity guarantee.

## Plan caching

Compiler cache keys include:

- exact query source;
- CompassQL and planner versions;
- ordered parameter types;
- planning limits;
- graph schema fingerprint.

A cached plan contains no graph values, parameter values, deadline,
cancellation state, or result rows. Changes that affect type or schema can
correctly produce a different plan.

## Portability boundary

CompassQL is not full openCypher, Neo4j Cypher, or ISO GQL.

It deliberately rejects:

- mutation (`CREATE`, `MERGE`, `DELETE`, `SET`, `REMOVE`);
- procedures and `CALL`;
- `LOAD CSV`;
- schema and administration commands;
- dynamic execution;
- arbitrary nested subqueries;
- user-defined functions;
- unbounded paths.

If a query must run in both Compass and another graph system, stay within the
[checked support matrix](../COMPASSQL_SUPPORT.md) and test both engines. Syntax
accepted by Neo4j is not automatically a Compass contract.

## Diagnostics and exit categories

Diagnostics include stable family codes and byte spans:

```text
CQL1xxx  source, syntax, unsupported surface, literal errors
CQL2xxx  scope, type, function, projection, union, path-shape errors
CQL3xxx  source, token, nesting, path, row, expansion, memory, time limits
CQL4xxx  parameter, runtime type, regex, arithmetic, invariant errors
```

At the CLI boundary:

- exit `2` means source/options/compile failure;
- exit `3` means graph loading failure;
- exit `4` means execution, limit, cancellation, or output failure.

These distinctions let automation decide whether to fix input, repair a graph,
or narrow execution.

## A practical selection rule

```text
Do I know the exact nodes, relations, and columns?
  |
  +-- no --> start with query / explain / path
  |
  `-- yes --> do I need repeatable rows or automation?
                |
                +-- no --> either surface can help
                |
                `-- yes --> use CompassQL + parameters + versioned output
```

## Related pages

- [CompassQL 1 reference](../COMPASSQL.md)
- [CompassQL support matrix](../COMPASSQL_SUPPORT.md)
- [Graph model](graph-model.md)
- [Integrating Compass](../guides/integrating-compass.md)

**Next step:** try a parameterized query from
[Integrating Compass](../guides/integrating-compass.md), then use `EXPLAIN` to
inspect its plan.
