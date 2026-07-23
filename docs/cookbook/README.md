# Compass cookbook

The cookbook contains short, outcome-focused recipes. Use it when you already
know the problem and want commands, interpretation guidance, and caveats in one
place.

> **Who this page is for:** everyday Compass users and integrators.
>
> **You will learn:** which recipe fits your task and how cookbook pages differ
> from full guides and references.
>
> **Prerequisites:** [Getting started](../getting-started.md).
>
> **Reading time:** 3 minutes.

## Pick a problem

| I need to… | Recipe |
| --- | --- |
| estimate the review surface of a symbol or change | [Impact analysis](impact-analysis.md) |
| map a subsystem or request/data flow | [Architecture discovery](architecture-discovery.md) |
| generate and query graphs in CI | [CI and automation](ci-and-automation.md) |
| diagnose install, build, query, provider, or history trouble | [Troubleshooting](troubleshooting.md) |

## Recipe format

Each recipe follows:

```text
Problem
  what you are trying to accomplish

Commands
  the smallest copyable workflow

Interpretation
  what the result does and does not prove

Variations
  common changes for nearby tasks

Safety / recovery
  boundaries, failures, and cleanup
```

For a complete learning path, use a [guide](../README.md#complete-a-task). For
exact options and schemas, use the [reference](../README.md#look-up-an-exact-contract).

## Quick recipes

### Find a concept

```bash
compass query "payment retry and idempotency"
```

Then explain one concrete result:

```bash
compass explain RetryPolicy
```

### Connect two boundaries

```bash
compass path ApiHandler PaymentRepository
```

Verify each hop in source.

### List direct callers

```bash
compass query --cql \
  "MATCH (caller)-[:CALLS]->(target)
   WHERE target.label = 'authorize_payment'
   RETURN caller.id, target.id
   ORDER BY caller.id
   LIMIT 100"
```

### Compare two commits

```bash
compass diff HEAD~1 HEAD
```

For scripts:

```bash
compass diff HEAD~1 HEAD --format json
```

### Build only structural code knowledge

```bash
compass extract . --code-only
```

### Create a reproducible graph for a commit

```bash
compass history build HEAD --code-only
compass history export HEAD \
  --format graph-json \
  --output target/head-graph.json
```

## Related pages

- [Explore a codebase guide](../guides/exploring-a-codebase.md)
- [Command reference](../reference/commands.md)
- [Graph model](../concepts/graph-model.md)

**Next step:** choose the recipe matching your current task and record the graph
revision/profile beside any saved result.
