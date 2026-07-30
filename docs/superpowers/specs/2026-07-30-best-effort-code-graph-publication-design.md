# Publish a usable code graph when individual records are invalid

**Status:** Approved by product direction, pending written review
**Date:** 2026-07-30
**Product:** Compass
**Schemas:** `compass.graph/1`, `compass.query/1`
**Content type:** Conceptual

## Summary

Compass will publish the largest structurally valid code graph it can derive from a repository. A malformed node, edge, producer relation, identity collision, or endpoint-kind combination will no longer abort the complete build. Compass will quarantine the invalid record, remove topology that depends on it, record bounded diagnostics, and validate the remaining graph before atomic publication.

The durable `compass.graph/1` contract remains strict. Query consumers will never need to handle dangling endpoints, unknown kinds, malformed provenance, or invalid source paths. Compass moves tolerance to the producer-to-publication boundary instead of weakening the artifact consumed by the command-line interface (CLI), Model Context Protocol (MCP) server, Visual Studio Code extension, and coding agents.

## Product decision

The primary product outcome is a queryable graph that helps a coding agent navigate a real repository. Complete graph rejection is worse than a disclosed omission when most symbols and relationships remain usable.

Compass therefore distinguishes two failure classes:

- **Record-level failure:** Quarantine the node or edge, record the omission, and continue
- **Document-level failure:** Abort publication because no safe query artifact can be produced

Default developer builds use best-effort publication. The existing strict normalizer and validator remain available to unit tests, producer qualification, and diagnostic tooling.

## Goals

1. Publish a queryable graph when individual producer records are invalid.
2. Preserve the strict `compass.graph/1` invariant set for every published artifact.
3. Maximize retained nodes and edges without fabricating replacement semantics.
4. Tell agents and operators that the graph is partial.
5. Make every quarantine decision deterministic across clean, cached, forced, and reordered builds.
6. Preserve atomic generation publication and the last-known-good graph.
7. Bound diagnostic volume on repositories that produce hundreds or thousands of invalid records.

## Non-goals

- Publishing dangling edges or unknown node and edge kinds
- Converting every unknown relation to `references`
- Guessing a replacement node kind solely to satisfy validation
- Treating a partial graph as complete
- Hiding producer defects from qualification reports
- Solving extraction and query latency in the same change
- Adding a second durable graph schema or a second graph artifact

## Terms

**Quarantine** means omitting an invalid raw or normalized record from the published graph and recording why Compass omitted it.

**Record-level failure** means a failure attributable to one raw node, normalized node, raw edge, or normalized edge. Incident edges also become record-level omissions when their endpoint is quarantined.

**Document-level failure** means a failure that prevents Compass from producing or publishing a structurally safe artifact. Examples include an unsupported schema, unsafe repository path, unreadable authoritative input, invalid file inventory, empty usable graph, serialization failure, and atomic publication failure.

**Partial graph** means a strictly valid graph that includes one or more quarantine diagnostics.

## Chosen architecture

Compass will add a best-effort publication path beside the existing strict path:

```text
resolved raw extraction
        |
        v
best-effort node normalization
  keep valid nodes
  quarantine invalid nodes and conflicting duplicates
        |
        v
best-effort edge normalization
  drop edges with quarantined endpoints
  quarantine malformed or unknown relations
        |
        v
typed record sanitization
  quarantine invalid normalized nodes
  cascade incident-edge removal
  quarantine invalid normalized edges
        |
        v
existing strict graph validator
        |
        v
atomic compass.graph/1 publication
```

`compass-model` continues to own strict validation. `compass-graph` owns quarantine and diagnostic accounting. `compass-core` uses the best-effort path for normal builds and reports the omission summary. `compass-query` converts partial-publication diagnostics into `incomplete_coverage` query diagnostics.

## Public and internal interfaces

The existing `normalize_v1` and `normalize_document_v1_with_inventory` functions keep their strict behavior. Existing strict tests and callers do not silently change semantics.

`compass-graph` will add best-effort variants that return a typed publication outcome:

```rust
pub struct PublicationOutcome {
    pub document: GraphDocument,
    pub omissions: PublicationOmissions,
}

pub struct PublicationOmissions {
    pub nodes: usize,
    pub edges: usize,
    pub identity_collisions: usize,
    pub examples_omitted: usize,
}
```

The concrete function names may follow existing crate naming conventions, but the strict and best-effort boundaries must remain explicit in the Rust API.

Normal `compass extract`, `compass update`, watch rebuilds, and history materialization will use best-effort publication. Producer-focused tests can continue to call the strict path.

## Deterministic quarantine algorithm

### Normalize nodes

Compass sorts raw nodes by a stable record key before normalization. The key includes raw identity, declared kind, qualified name, portable source path, source range, and canonical serialized attributes.

For each raw node:

1. Normalize the node with the current typed normalizer.
2. If normalization fails, quarantine the raw node and mark its raw identity unavailable for edge remapping.
3. If normalization succeeds with a new stable identity, retain it.
4. If the stable identity already exists and the records are compatible, merge them with the current evidence-preserving merge.
5. If the stable identity already exists and the records conflict, choose one deterministic survivor and quarantine the other raw record.

The survivor ranking is:

1. Exact Abstract Syntax Tree (AST) evidence
2. Other exact source-backed evidence
3. Inferred source-backed evidence
4. Sourceless evidence with an exact wiring site
5. Canonical serialized record order as the final tie-breaker

Compass does not remap edges from a quarantined conflicting raw record to the survivor. Those edges are omitted because their intended semantic owner is ambiguous.

### Normalize edges

Compass sorts raw edges by source identity, raw relation, target identity, relationship site, occurrence rule, and canonical serialized attributes.

For each raw edge:

1. Omit the edge if either raw endpoint was quarantined.
2. Omit the edge if an endpoint has no normalized identity.
3. Normalize the relation, provenance, source anchor, and stable edge identity.
4. Quarantine the edge if normalization fails, including unknown raw relations such as an unregistered producer-specific relation.
5. Merge compatible duplicate edges with the current evidence-preserving merge.
6. Quarantine conflicting duplicate edges instead of aborting the graph.

Compass will not map an unknown relation to a generic edge kind. A missing edge conveys less false information than an invented relationship.

### Sanitize typed records

The model layer will expose structured record validation helpers. The publication path must not parse validator error strings.

Sanitization proceeds in this order:

1. Validate document metadata and file inventory. Any failure remains fatal.
2. Validate each normalized node independently.
3. Quarantine invalid nodes.
4. Remove every edge incident to a quarantined node.
5. Validate each remaining edge against its endpoints.
6. Quarantine invalid edges.
7. Recompute route resolution when a quarantined edge was a route stage.
8. Run the existing full strict validator.

If the final validator still fails, publication aborts. This catches implementation defects in the sanitizer rather than exposing an invalid artifact.

### Preserve a usable minimum

Best-effort publication has no percentage-based omission threshold. A repository can contain unsupported or malformed regions while retaining a valuable graph elsewhere.

Publication still fails when no usable symbol, file, route, component, domain, or database node remains. Existing last-known-good generation behavior remains authoritative when that failure occurs.

## Diagnostics

Compass records quarantine outcomes in `graph.diagnostics` with warning severity:

- `publication_omitted_node`
- `publication_omitted_edge`
- `publication_identity_collision`
- `publication_omission_summary`

Each example includes the original failure reason, raw or stable identity, portable source or wiring site when available, and related retained identities when applicable.

Compass stores at most 100 node examples, 100 edge examples, and 100 collision examples. The summary records total omitted nodes, omitted edges, identity collisions, and examples excluded by the diagnostic cap. Counts remain exact even when examples are bounded.

The CLI prints one concise success warning:

```text
Compass published a partial graph: 2 nodes and 801 edges omitted.
Run `compass diagnose publication` for details.
```

The first implementation may expose the details through existing graph diagnostics before adding a dedicated `diagnose publication` command. The CLI must not advertise that command until it exists.

## Query behavior

Every query continues to operate on a strictly valid graph. Traversal code does not add malformed-record branches.

When graph diagnostics contain a publication omission summary, search, callers, callees, impact, explore, node trail, and call graph responses include one `incomplete_coverage` diagnostic. The message reports omission counts and states that absent topology may reflect quarantine.

Queries do not append every record-level diagnostic. Agents can inspect graph diagnostics or a diagnostic command when they need examples.

## Framework-specific behavior

Framework routes, handlers, middleware, events, jobs, schemas, and database facts follow the same policy:

- A valid route node remains queryable when an unrelated route fails.
- A route edge with an invalid handler endpoint is omitted.
- The route stage becomes unresolved after its edge is omitted.
- The route node retains framework evidence and its relationship site.
- A malformed domain relationship does not remove valid domain entities.

Compass must recompute route resolution after quarantine so route details never claim exact resolution without a retained `routes_to` edge.

## Atomic publication and history

Best-effort normalization finishes before Compass writes the candidate generation. Graph JSON, Program intermediate representation (IR), manifest, build state, and generation pointer retain their current atomic publication boundary.

A valid partial graph may replace the active generation. A document-level failure cannot replace it. History builds record the same deterministic omission diagnostics for the same source revision and configuration.

## Observability

Build statistics will expose:

- retained node count
- retained edge count
- omitted node count
- omitted edge count
- identity collision count
- partial graph status

Profiling will measure node normalization, edge normalization, typed sanitization, strict final validation, and diagnostic serialization separately. Quarantine must not require repeatedly cloning the complete graph.

## Test strategy

### Model tests

- Validate nodes and edges independently through structured helpers.
- Keep the full strict validator behavior unchanged.
- Prove invalid endpoint kinds remain invalid in durable artifacts.

### Graph normalization tests

- Strict normalization still rejects unknown relations and missing wiring sites.
- Best-effort normalization omits one unknown edge and publishes the remaining graph.
- Best-effort normalization omits one invalid node and every incident edge.
- Stable identity collisions retain the same survivor under reversed input order.
- Invalid endpoint-kind edges become bounded diagnostics.
- Route resolution becomes unresolved when quarantine removes `routes_to`.
- Sanitized output passes the existing strict validator.
- Repeated and reordered builds serialize to identical bytes.

### Pipeline tests

- Normal extract and update publish partial graphs successfully.
- The CLI reports omission counts without reporting a failed build.
- An empty usable graph remains a failure.
- A failed document-level publication preserves the active generation.
- Cached and forced builds produce identical partial graphs.

### Query tests

- Every typed operation includes one `incomplete_coverage` diagnostic for a partial graph.
- Query results remain usable and bounded.
- Complete graphs do not receive a partial-graph diagnostic.

## Heavy-framework qualification

Compass will rerun the pinned official repositories used in the 2026-07-29 qualification:

| Repository | Expected outcome after this change |
|---|---|
| Django | Publishes the same valid topology plus deterministic diagnostics |
| Spring Framework | Quarantines the unwired placeholder and publishes the remaining graph |
| Angular | Publishes within the existing 10-minute observation ceiling or remains a separately reported performance blocker |
| ASP.NET Core | Quarantines the conflicting Razor record and publishes the remaining graph |
| Rails | Omits invalid endpoint-kind edges and publishes the remaining graph |
| Laravel Framework | Omits the unknown `bound_to` edge and publishes the remaining graph |
| Bevy | Quarantines the conflicting Rust module record and publishes the remaining graph |

For each successful publication, qualification records wall time, peak resident memory, retained and omitted record counts, validation errors, graph hash, route counts, and a real search or callers query.

This change is complete only when Spring Framework, ASP.NET Core, Rails, Laravel Framework, and Bevy publish queryable strict-valid graphs. Angular performance remains in scope for the broader production-readiness goal but is not disguised as a validation failure.

## Documentation changes

Update the graph model, output reference, command reference, operations guide, and troubleshooting guide:

- Explain that default publication is best-effort.
- Define partial graph and quarantine.
- List document-level hard failures.
- Explain diagnostic caps and exact summary counts.
- Tell agents to verify critical paths in source when coverage is incomplete.

The original Code Graph v1 design will receive a short amendment that replaces whole-build failure for record-level defects with this approved quarantine contract. Historical text remains otherwise intact.

## Acceptance criteria

1. Default builds quarantine record-level failures and continue.
2. Every published graph passes the unchanged strict validator.
3. Invalid nodes never leave dangling edges.
4. Unknown relations never enter the durable edge vocabulary.
5. Identity collision survivors and omission diagnostics are deterministic.
6. Query responses disclose partial publication.
7. Diagnostic examples are bounded and summary counts are exact.
8. Last-known-good publication behavior remains intact for document-level failures.
9. The five heavyweight repositories that previously failed validation publish queryable graphs.
10. Existing strict normalization APIs and tests remain available for producer qualification.
