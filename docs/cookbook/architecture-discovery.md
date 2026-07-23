# Cookbook: architecture discovery

These recipes help map a subsystem, request flow, data flow, or boundary in an
unfamiliar repository.

> **Who this page is for:** developers onboarding, debugging across modules, or
> preparing architecture reviews.
>
> **You will learn:** community-first, entry-to-side-effect, hub, boundary, and
> exact-pattern recipes.
>
> **Prerequisites:** a fresh `compass-out/`.
>
> **Completion time:** 10–30 minutes.

## Recipe 1: map one subsystem

### Problem

You need a compact map of authentication.

### Workflow

```bash
sed -n '1,220p' compass-out/GRAPH_REPORT.md
compass query "HTTP authentication token verification and sessions"
compass explain AuthMiddleware
```

Write:

```text
Entry:
Core services:
State:
External boundary:
Likely community:
Unverified/ambiguous:
```

Then verify those source files.

## Recipe 2: trace entry to side effect

### Problem

You need the path from an API handler to a database or external provider.

### Commands

```bash
compass query "checkout request handling"
compass query "payment persistence gateway"
compass path CheckoutHandler PaymentRepository
```

For each hop:

```text
node A --relation [provenance]--> node B
source A:
source B:
condition/dynamic caveat:
```

If the path ends at an abstraction, query its implementers or concrete clients.

## Recipe 3: find cross-subsystem coupling

### Problem

You want edges crossing community boundaries.

### CompassQL

```bash
compass query --cql \
  'MATCH (source)-[edge]->(target)
   WHERE source.community <> target.community
   RETURN source.community, source.id, type(edge),
          target.community, target.id, edge.confidence
   ORDER BY source.community, target.community
   LIMIT 500' \
  --format json \
  --output target/cross-community.json
```

Interpret repeated cross-community relationships as a boundary worth
investigating. One edge may be a legitimate adapter.

## Recipe 4: inspect hubs

### Problem

The report identifies a god node.

### Commands

```bash
compass explain AppContext
compass affected AppContext --depth 2
```

Ask:

- Is it a true composition root?
- Is it a broad interface?
- Is it a generic built-in/noisy label?
- Are incoming and outgoing relations balanced?
- Does it connect many communities?
- Would changing it create widespread review risk?

High degree is evidence of connectivity, not automatically a design smell.

## Recipe 5: find cycles

Start with `GRAPH_REPORT.md` diagnostics. If import-cycle data is present,
inspect each edge direction and source.

For an exact bounded pattern, use the relation vocabulary in your graph. Do not
write an unbounded CompassQL path; set an explicit maximum no greater than 32.

Example shape:

```cypher
MATCH p=(module)-[:IMPORTS_FROM*2..8]->(module)
RETURN module.id, p, length(p)
LIMIT 100
```

Verify supported path semantics in [CompassQL 1](../COMPASSQL.md).

## Recipe 6: create a shareable subsystem brief

Use:

```text
Subsystem
  One sentence describing responsibility.

Entry points
  Public handlers/commands/jobs.

Core path
  3–7 graph hops with relation and provenance.

State and integrations
  Files, DBs, queues, providers, external services.

Internal modules/communities
  Major graph-derived groups, qualified as hypotheses.

Change hotspots
  Hubs and incoming impact neighborhoods.

Uncertainty
  Ambiguous/dynamic/ignored/generated/external behavior.

Evidence
  Commit/profile, commands, and source files.
```

This is more useful than attaching an unexplained screenshot of the whole
graph.

## Related pages

- [Explore a codebase](../guides/exploring-a-codebase.md)
- [Graph model](../concepts/graph-model.md)
- [Impact analysis](impact-analysis.md)

**Next step:** produce one subsystem brief and ask a maintainer to verify its
boundary and uncertainty, not just its terminology.
