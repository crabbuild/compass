## Context

See `proposal.md` for motivation. C-014 stores exact node and relation payloads
under repository/generation keys and activates them atomically. The current
`compass-query` implementation already defines deterministic structural semantics
and bounds over JSON/store backends. C-013 froze the semantic corpus, scale
profiles, measurements, and numeric gates before C-014/C-015 results existed.

The SurrealDB dependency must remain absent from default Compass closures, and
the optional adapter cannot expose raw database language to callers.

## Goals / Non-Goals

**Goals:**

- Reuse the shared `compass.query/1` node, relation, path, evidence, diagnostic,
  and limit contracts for both engines.
- Execute adjacency and record lookup as generation-pinned native Surreal reads,
  with Rust coordinating deterministic bounded traversal.
- Keep equivalence comparison canonical, timing-independent, and strict.
- Retain reproducible host-specific measurement evidence for every C-013 gate.

**Non-Goals:**

- Switching the default CLI or MCP engine to SurrealDB.
- Adding remote SurrealDB configuration, authentication, arbitrary SurrealQL, or
  presentation-specific response types.
- Moving ranking, traversal semantics, or public bounds into the database.
- Treating a single-host measurement as a universal performance claim.

## Decisions

### Share result contracts, keep execution asynchronous and adapter-owned

The adapter returns the existing model-layer query records and uses the same
limit vocabulary. Its API remains asynchronous because embedded Surreal engines
are asynchronous. The current engine supplies a bounded structural comparison
route in `compass-query`; the Surreal SDK is not pulled into `compass-query`'s
default dependencies.

An async backend trait inside `compass-query` was rejected because it would force
Surreal lifecycle and features into the default query crate. Materializing the
complete projection and reusing the current engine was rejected because that is
not a native read and violates the bounded-allocation purpose of C-015.

### Pin the active generation once

Each operation reads and validates the repository pointer and complete manifest
once, producing an internal generation selector. Every subsequent statement
binds repository and generation explicitly. The selector is revalidated before
publishing the response; a changed pointer yields a typed generation-change
error. This prevents mixed-generation results while preserving immutable
historical records.

Keeping a long database transaction open for every read was rejected because it
adds unnecessary engine coupling and contention. Immutable generation records
plus start/end pointer validation provide the required coherent outcome.

### Static family queries with bounded Rust traversal

Node lookup and each of the five closed relation families use static statements,
parameter-bound values, explicit canonical ordering, and remaining-limit-plus-one
sentinels. Rust merges family results by stable edge ID, filters the closed kind
sets, and performs BFS with the existing tie-breaking and evidence-quality rules.
This preserves parallel edges and self-loops without relying on database
iteration order.

A caller-provided table parameter is rejected even though SurrealDB supports
`type::table`; only code-owned family constants may select a table. One large
recursive SurrealQL statement was rejected because it makes independent node,
edge, depth, and response accounting difficult to audit.

### Generation-bound opaque cursors

The page cursor is a versioned canonical payload containing repository digest,
generation, operation, ordering version, and last stable identity, authenticated
with a content digest. It reveals no raw database record ID and is validated
before reads. Query-side ordering uses stable Compass identity and requests
`page_size + 1` to distinguish completion from truncation.

Offset pagination was rejected because concurrent pointer changes and large
offset scans weaken determinism and work bounds.

### Differential qualification is a hard oracle

Tests build one validated graph, project it to Mem, run the current and Surreal
operations with identical exact identities and limits, canonicalize only
transport-irrelevant metadata, and require byte-equal semantic results. Separate
persistent-engine/scale qualification records output digests, page boundaries,
timings, RSS, and ratios. A mismatch or missed threshold is a failure, not a
tolerance entry.

## Risks / Trade-offs

- **[Multiple family queries increase call count]** → record query-call counts,
  merge deterministically, and use the C-013 native-value gate as a falsifier.
- **[Start/end pointer validation may reject an otherwise coherent historical
  result after activation]** → prefer explicit retryable failure over returning a
  result the caller could mistake for the current generation.
- **[Current search ranking is broader than structural exact lookup]** → qualify
  exact identity/name structural operations; do not claim fuzzy/full-text parity
  without a separate contract and index design.
- **[Medium/large qualification is resource intensive]** → keep generated graphs,
  stores, binaries, and raw logs disposable under an explicit target directory;
  retain only bounded manifests and summarized raw samples.
- **[Optional SDK features can leak into default binaries]** → extend the
  dependency-closure and binary proof gate after every dependency edit.

## Migration Plan

This is additive and disabled by default. Land the shared contract and current
comparison route first, then the feature-gated Surreal reader, then retained
qualification evidence. Rollback removes the optional route and evidence without
changing canonical graph formats, default query behavior, or active historical
generations.
