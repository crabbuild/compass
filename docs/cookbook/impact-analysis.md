# Cookbook: impact analysis

Use these recipes to estimate what may depend on a symbol, file, or change.

> **Who this page is for:** implementers and reviewers preparing a change.
>
> **You will learn:** reverse-impact traversal, direct-caller queries,
> historical topology diffs, and how to turn results into a review checklist.
>
> **Prerequisites:** a current graph or versioned history.
>
> **Completion time:** 5–15 minutes.

## Recipe 1: impact from one symbol

### Problem

You plan to change `TokenVerifier` and want a bounded review scope.

### Commands

```bash
compass explain TokenVerifier
compass affected TokenVerifier --depth 3
```

### Interpret

`explain` shows immediate incoming/outgoing context. `affected` walks incoming
impact-relevant relations:

```text
ApiMiddleware --CALLS--> TokenVerifier

change TokenVerifier
        |
        `--> review ApiMiddleware
```

The result is a review queue, not a required edit list.

### Variations

Narrow to one relation:

```bash
compass affected TokenVerifier --relation calls --depth 2
```

Use a saved graph:

```bash
compass affected TokenVerifier --graph target/baseline.json --depth 3
```

## Recipe 2: exact direct callers

### Problem

You need a deterministic list for automation.

### Command

```bash
compass query --cql \
  'MATCH (caller)-[edge:CALLS]->(target)
   WHERE target.label = $target
   RETURN caller.id, edge.confidence, target.id
   ORDER BY caller.id
   LIMIT 500' \
  --param target=TokenVerifier \
  --format json \
  --output target/token-verifier-callers.json
```

### Interpret

Review the schema tag, then distinguish direct and resolved evidence by
`edge.confidence`.

If labels repeat, first discover the stable target ID and query by `target.id`.

## Recipe 3: downstream dependencies

### Problem

You want to know what a service calls or uses.

### Command

```bash
compass query --cql \
  'MATCH (source)-[edge:CALLS|USES|IMPORTS_FROM]->(dependency)
   WHERE source.id = $source
   RETURN type(edge), dependency.id, edge.confidence
   ORDER BY type(edge), dependency.id
   LIMIT 500' \
  --param source='"src/auth.rs::TokenVerifier"' \
  --format table
```

Parameter parsing accepts JSON scalars; quote a string explicitly when shell
content could be mistaken for another JSON type.

## Recipe 4: compare topology across commits

### Problem

You need to see structural additions/removals without report noise.

### Commands

```bash
compass history build HEAD~1 --code-only
compass history build HEAD --profile-from HEAD~1
compass diff HEAD~1 HEAD --topology-only
```

For automation:

```bash
compass diff HEAD~1 HEAD --topology-only --format json \
  > target/topology-diff.json
```

### Interpret

The profile-from step makes extraction semantics comparable. Added/removed
edges show static topology change; they do not prove runtime traffic changed.

## Recipe 5: PR review checklist

### Problem

A PR changes several files and you want graph-informed review.

### Workflow

1. Record changed files:

   ```bash
   git diff --name-only BASE...HEAD
   ```

2. Query/explain key changed symbols.
3. Run `affected` for public or hub symbols.
4. Compare exact commit graphs if both revisions are available.
5. Inspect cross-community edges.
6. Add tests/configuration/consumers to the review list.

Checklist:

```text
[ ] Direct callers reviewed
[ ] Importers/consumers reviewed
[ ] Implementers/inheritors reviewed
[ ] Tests and fixtures reviewed
[ ] Configuration/schema dependencies reviewed
[ ] Ambiguous edges verified manually
[ ] Dynamic/reflection/external behavior considered
[ ] Graph profile and revision recorded
```

## Common false conclusions

| Result | Do not conclude |
| --- | --- |
| Node appears in `affected` | It must be edited |
| Node does not appear | It cannot be affected dynamically |
| Edge is `EXTRACTED` | It always executes |
| Edge is `INFERRED` | It is unreliable or model-generated |
| One shortest path exists | It is the only runtime path |
| Topology unchanged | Behavior is unchanged |

## Recovery

If results are empty:

- confirm graph freshness and requested revision;
- explain the seed to confirm identity;
- query by file/module first;
- inspect ignore/generated code;
- confirm relation names;
- use `--at` for historical rather than a current graph.

## Related pages

- [Explore a codebase](../guides/exploring-a-codebase.md)
- [Provenance](../concepts/provenance.md)
- [Versioned history](../guides/versioned-history.md)

**Next step:** convert the impact output into a human review checklist and
verify the highest-risk relations in source.
