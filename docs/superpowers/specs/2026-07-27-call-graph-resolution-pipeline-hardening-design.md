# Call-Graph Resolution Pipeline Hardening Design

**Date:** 2026-07-27

**Status:** Approved

**Implementation root:** `/Users/haipingfu/graphify/compass`

## Purpose

Compass call graphs must remain responsive on large repositories without hiding
partial coverage or moving traversal semantics into the VS Code extension. The
first hardening pass made structural traversal adjacency-indexed and stopped
blocking every editor request on Program IR. This design completes that work:

1. paginate every excluded continuation without making responses unbounded;
2. share the bounded traversal implementation with Program IR;
3. establish a release performance gate;
4. report real resolver progress;
5. restore Program IR as nonblocking progressive enrichment; and
6. serve requests from compact, persistent indexes with a small editor-session
   cache.

All six changes will land in the existing draft pull request as separate,
reviewable commits in the order above.

## Goals

- A fresh Compass process resolves a structural call graph in at most 500 ms
  when `compass update` has already published the compact index.
- A repeated request in the same VS Code repository session resolves in at
  most 100 ms from the session cache.
- Cached Program IR enrichment completes in at most one second.
- The first progress event is visible within 100 ms.
- Cancellation is acknowledged within 100 ms and no superseded result is
  rendered.
- Every truncated frontier branch remains reachable through deterministic
  pagination.
- Structural results render before optional semantic enrichment.
- Existing `compass call-graph --format json`,
  `compass.program.call_graph/1`, and supported JSON consumers remain
  compatible.
- Cache corruption, cache-write failures, stale cursors, and enrichment
  failures have explicit, recoverable behavior.

The release benchmark uses the retained Compass corpus with 26,567 nodes and
63,204 edges. Cache-miss migration and first index construction are measured
separately from the 500 ms indexed cold-process budget.

## Non-goals

- Add a long-lived resolver daemon.
- Move root resolution, traversal, evidence merging, or limits into VS Code.
- Change extraction or call-edge inference semantics.
- Make Program IR a prerequisite for opening a call graph.
- Change the legacy `compass program call-graph` response schema.
- Promise that all supported languages have identical call precision.
- Add a general-purpose cache framework for unrelated Compass commands.

## Chosen architecture

Compass uses versioned, signature-validated on-disk projections plus a small
VS Code session LRU. It continues to spawn a fresh CLI process for each cache
miss. A resident daemon was rejected because its lifecycle, upgrade, crash
recovery, workspace-trust, and memory ownership costs are disproportionate to
this feature. Extension-side traversal was rejected because Rust is the
canonical owner of deterministic traversal and evidence semantics.

```text
graph.json ───────> compact structural call index ──┐
                                                     ├─> compass call-graph
program.json ─────> compact Program IR call index ──┘          │
                                                               ├─ structural result
VS Code request ──> repository-session LRU ── miss ────────────┤
                                                               ├─ progress events
                                                               └─ enrichment result
```

The structural result is terminal for the structural request and immediately
renderable. Enrichment is a separate request and never returns the panel to the
full-screen loading state.

## Component boundaries

### Shared traversal engine

`compass-analysis` will expose one internal direction-aware breadth-first
traversal implementation used by structural and Program IR call graphs. It
consumes:

- a root identity;
- incoming and outgoing adjacency;
- direction;
- depth;
- maximum nodes;
- maximum edges; and
- a continuation page request.

It produces selected node IDs, selected edge indexes, the complete ordered
frontier metadata needed for the requested page, and truncation counts.
Evidence conversion and response shaping remain in their respective builders.

The engine must:

- never scan the complete edge set once per visited node;
- stop queue admission at the node bound;
- stop edge admission at the edge bound;
- handle self-edges and cycles without duplicate work;
- sort before applying bounds;
- produce the same order for identical inputs; and
- expose cancellation checkpoints no more than 100 ms apart during index and
  traversal work.

### Compact structural index

`compass-analysis` will define `compass.call_index/1`, stored next to the
existing graph query caches at
`cache/.graph.json.compass-call-index-v1`. It contains only data required by
call-graph resolution:

- graph artifact fingerprint;
- callable nodes with stable ID, label, kind, file, inclusive lines, and
  available evidence metadata;
- callable IDs grouped by normalized source file and ordered by source range;
- structural call edges;
- incoming and outgoing adjacency; and
- deterministic edge and frontier ordering keys.

`compass update` attempts to publish this index atomically after publishing a
valid graph. Index publication failure does not invalidate the graph; a
call-graph request may build the index lazily from the valid graph, use the
in-memory result, and retry the atomic cache write.

The cache header includes a cache magic, schema version, source artifact
length, source modification timestamp, and the SHA-256 captured when the index
was built. Fast cache validation uses the same length and modification-time
signature as the existing graph query cache. When a verified Compass build
seal is available, its graph digest must equal the stored SHA-256. Lazy builds
compute the digest while reading the source graph. A mismatch is a cache miss,
never a partial read.

### Compact Program IR index

`compass-analysis` will define `compass.program_call_index/1`, stored at
`cache/.program.json.compass-call-index-v1`. It contains:

- the Program IR artifact signature;
- functions keyed by Program IR symbol and structural `graph_node_id`;
- source byte anchors;
- resolved, inferred, ambiguous, and unresolved call edges;
- exact call-site anchors and evidence IDs; and
- incoming and outgoing adjacency.

The first valid Program IR load still performs the existing size, schema,
validation, and canonical-byte checks. It then writes the compact index
atomically. Subsequent matching requests load the projection rather than
deserializing, validating, and canonically reserializing the entire
`program.json`.

Program cache failure cannot invalidate a valid structural result.

### VS Code repository-session cache

The extension will reuse the existing bounded LRU pattern. It holds at most
eight entries per repository and evicts until the estimated serialized total
is at most 16 MiB. The call-graph LRU key is:

```text
repository ID
+ graph artifact fingerprint
+ normalized root
+ direction
+ depth
+ node/edge limits
+ continuation cursor
+ evidence layer
```

The cache stores validated response objects, not raw stdout. A graph
fingerprint change invalidates every entry for that repository.

## Continuation pagination contract

`compass.call_graph/1` gains additive fields:

```json
{
  "artifactFingerprint": "sha256:...",
  "continuationPage": {
    "returned": 100,
    "omitted": 2400,
    "nextCursor": "opaque-or-null"
  }
}
```

The existing `continuations` array remains the current page. The default page
size is 100 and cannot exceed the request's node bound.

`compass call-graph` accepts:

```text
--continuation-cursor <OPAQUE_TOKEN>
```

The token encodes:

- cursor contract version;
- artifact fingerprint;
- normalized root identity;
- direction;
- depth;
- node and edge limits;
- evidence layer; and
- the last deterministic frontier ordering key.

The token is base64url-encoded canonical data. It is not trusted: decode,
schema, fingerprint, and request-identity checks happen before traversal. A
tampered or stale token returns a typed cursor error and no graph result.

The viewer presents:

- up to twenty branch buttons initially;
- the existing “show all on this page” behavior;
- an exact omitted count; and
- “Load more branches” when `nextCursor` is present.

Symbol-specific expansion remains available. Pagination adds reachability for
frontier symbols omitted by response bounds; it does not replace branch
expansion.

## Progress and timing protocol

`compass call-graph --format json` retains its current single-response
behavior. `--format jsonl` emits `compass.call_graph.events/1` records:

```json
{"type":"progress","phase":"loading_structural_index","elapsedMs":4,"terminal":false}
{"type":"progress","phase":"locating_symbol","elapsedMs":7,"terminal":false}
{"type":"progress","phase":"tracing_calls","elapsedMs":11,"terminal":false}
{"type":"result","graph":{},"elapsedMs":13,"terminal":true}
```

Allowed structural phases are:

- `loading_structural_index`
- `locating_symbol`
- `tracing_calls`
- `serializing_result`

Allowed enrichment phases are:

- `loading_program_index`
- `joining_evidence`
- `serializing_result`

Every successful stream has exactly one terminal result. Every failed stream
has exactly one terminal error. Cancellation may end the process without a
terminal record, and the host treats that as cancellation only when it
requested the abort.

The JSON response gains additive resolver timing metadata:

```json
{
  "timingsMs": {
    "indexLoad": 4,
    "rootResolution": 3,
    "traversal": 4,
    "enrichment": 0,
    "resolverTotal": 11
  }
}
```

The JSONL terminal envelope additionally reports serialization and end-to-end
elapsed time because those values are only known after the response has been
encoded. VS Code maps progress phases to the three user-facing steps and
records slow requests in the Compass output channel with artifact fingerprint,
artifact size, direction, depth, result counts, cache hit/miss, and phase
timings.

## Progressive enrichment

After a structural result is visible, VS Code automatically starts a
cancellable enrichment request when Program IR is available.

The webview shows a compact “Adding semantic evidence…” status without hiding
the structural graph. An enrichment result may merge only when all of these
still match:

- repository ID;
- request generation;
- root identity;
- direction;
- depth;
- graph artifact fingerprint; and
- structural response schema.

Merge behavior preserves structural nodes and relationships, adds exact byte
anchors and evidence IDs, adds Program IR-only unresolved or ambiguous calls,
deduplicates call sites, and updates coverage counts and evidence-layer labels.

An enrichment error leaves the structural graph visible, changes the compact
status to a nonfatal limitation, and writes diagnostic detail to the output
channel. Retry applies only to enrichment and does not discard the structural
graph.

## Error handling and cancellation

- Missing structural index: build from a valid graph and continue.
- Corrupt or incompatible structural index: ignore it, rebuild, and replace it
  atomically when possible.
- Structural cache write failure: return the computed structural graph and log
  that persistence failed.
- Missing Program IR: finish with structural coverage.
- Invalid Program IR: keep the structural graph visible and report enrichment
  unavailable.
- Corrupt or incompatible Program IR index: validate the source Program IR and
  rebuild the projection.
- Stale cursor: return a typed error that lets the viewer restart at page one.
- Superseded request: abort its process and ignore every late event.
- Panel disposal: abort structural and enrichment processes and remove
  listeners.
- Limit breach: return a bounded, explicitly partial response with pagination
  metadata; never label it complete.

## Compatibility

- Existing JSON call-graph requests remain valid.
- New JSON fields are additive within `compass.call_graph/1`.
- The viewer accepts responses without new fields during the extension/CLI
  compatibility window.
- `compass.program.call_graph/1` remains unchanged.
- Cache formats have independent versions and are safe to delete.
- A newer unsupported cache is a cache miss, not a CLI compatibility failure.
- JSONL progress uses a new schema and is capability-advertised before VS Code
  selects it.

## Performance qualification

Add a release-mode call-graph qualification script and CI gate. It records:

- Compass commit and version;
- operating system, architecture, CPU, memory, and Rust toolchain;
- graph and Program IR fingerprints and sizes;
- cache-miss migration time;
- indexed cold-process structural time;
- repeated warm structural time;
- cached enrichment time;
- continuation-page time;
- cancellation acknowledgement time;
- peak resident memory;
- output byte size; and
- deterministic result digest.

The controlled fixture budgets are:

- indexed cold-process structural: at most 500 ms;
- VS Code session-cache response: at most 100 ms;
- cached Program IR enrichment: at most one second;
- first progress event: at most 100 ms; and
- cancellation acknowledgement: at most 100 ms.

The benchmark fails on output-digest drift, missing terminal events, response
bounds violations, unreachable continuation pages, or a median latency
regression above ten percent from the approved baseline.

The normal unit suite keeps deterministic bounds and ordering assertions. It
does not use a strict wall-clock assertion for a large synthetic graph.

## Test strategy

### Rust

- Shared traversal contract tests run against structural and Program IR
  adapters for callers, callees, both, depth, node/edge bounds, pagination,
  cycles, self-edges, and deterministic ordering.
- Pagination tests collect all pages and prove every frontier continuation
  appears exactly once.
- Cursor tests cover tampering, schema mismatch, request mismatch, stale graph
  fingerprints, and deterministic replay.
- Structural and Program IR index tests cover round-trip, deterministic bytes,
  corruption, version mismatch, stale signatures, interrupted writes, and
  concurrent readers.
- CLI tests cover JSON compatibility, JSONL phase order, exactly one terminal
  event, lazy migration, and nonfatal cache-write failure.

### TypeScript and viewer

- Process-manager tests cover progress parsing, cancellation, malformed
  streams, duplicate terminal events, and output bounds.
- Panel tests cover session-cache hits, fingerprint invalidation, stale
  generations, pagination requests, structural-first rendering, and nonfatal
  enrichment errors.
- Viewer tests cover real active-step progression, omitted counts, loading the
  next continuation page, enrichment status, and evidence merging without
  duplicates.

### Release qualification

- Benchmark the retained Compass fixture in release mode.
- Record cold, warm, migration, enrichment, pagination, cancellation, memory,
  bytes, and digests.
- Retain machine-readable results as CI artifacts.

## Commit sequence

The expanded pull request uses six commits:

1. `feat(call-graph): paginate truncated continuations`
2. `perf(call-graph): share bounded Program IR traversal`
3. `test(call-graph): add performance qualification`
4. `feat(call-graph): stream resolver progress`
5. `perf(call-graph): cache progressive Program IR enrichment`
6. `perf(call-graph): persist compact call indexes`

Each commit must independently pass its focused tests. The final branch must
pass the complete Rust workspace checks, JavaScript tests, typecheck, extension
build, viewer asset verification, release benchmark, and `git diff --check`.
