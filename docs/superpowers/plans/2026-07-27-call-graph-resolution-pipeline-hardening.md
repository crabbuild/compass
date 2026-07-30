# Call-Graph Resolution Pipeline Hardening — Implementation-First Plan

**Goal:** Expand PR #74 with six production-focused commits that make Compass
call graphs responsive, progressive, deterministic, and resilient on large
repositories.

**Implementation style:** Implement each production slice completely, then add
and run focused validation before committing it. Tests are commit-boundary
qualification, not the driver or structure of the implementation.

**Target branch:** `codex/harden-call-graph-resolution`

**Performance fixture:** 26,567 nodes and 63,204 edges

## Product model

Compass has two first-class call-graph surfaces:

- `compass call-graph` provides a fast, language-neutral structural result and
  may enrich that result with Program IR.
- `compass program call-graph` provides the dedicated Program IR call graph and
  remains an active surface for future semantic enhancements.

They share traversal and index infrastructure, but their response contracts
remain independent. This PR does not deprecate, replace, or freeze the Program
IR call graph.

## Current implementation context

### Rust analysis

`crates/compass-analysis/src/universal_call_graph.rs` currently:

- scans all graph nodes and structural `calls` edges for every CLI process;
- resolves the cursor against callable graph nodes;
- builds incoming/outgoing adjacency for that request;
- traverses within node and edge bounds;
- caps the continuation set, making some excluded branches unreachable; and
- optionally enriches from a fully loaded `AnalysisBundle`.

`crates/compass-analysis/src/call_graph.rs` currently:

- constructs the Program IR node and edge collections for every request;
- scans `all_edges` for each visited node; and
- owns a separate breadth-first traversal implementation.

The structural traversal hardening already in PR #74 avoids the worst
per-visited-node scan for `compass call-graph`. This plan completes the work
without discarding that implementation.

### CLI

`crates/compass-cli/src/call_graph_commands.rs` currently:

- loads and validates `graph.json` on each request;
- loads the full canonical Program IR when `--program` is supplied;
- supports only `--format json`; and
- returns one buffered response after all resolution is complete.

`crates/compass-cli/src/bin/compass.rs` already has direct streaming routes for
commands such as watch. `crates/compass-cli/src/ide_contract.rs` and
`editors/vscode/src/cli/processManager.ts` already establish bounded JSONL
parsing patterns, but their generic progress event does not carry a call-graph
result.

### VS Code and viewer

`editors/vscode/src/views/callGraphPanel.ts` currently:

- starts a fresh structural CLI process for every root, direction, or
  expansion request;
- has one request controller;
- does not request Program IR enrichment; and
- reports only terminal errors.

`editors/vscode/src/webviews/callGraph.tsx` renders a static three-step loading
screen. It does not receive real phases. The shared viewer shows only the
continuations present in the response and has no pagination or enrichment
status.

### Artifact and build pipeline

The canonical artifacts remain:

```text
compass-out/graph.json
compass-out/program.json
```

The new disposable projections will be:

```text
compass-out/cache/.graph.json.compass-call-index-v1
compass-out/cache/.program.json.compass-call-index-v1
```

`crates/compass-core/src/pipeline.rs` publishes the canonical artifacts and
`crates/compass-core/src/build_state.rs` records their verified SHA-256 seals.
Index publication must be best-effort and must never invalidate a successfully
published canonical artifact.

## Non-negotiable contracts

- Indexed cold-process structural resolution: at most 500 ms.
- VS Code repository-session cache hit: at most 100 ms.
- Cached Program IR enrichment: at most 1,000 ms.
- First progress event: at most 100 ms.
- Cancellation acknowledgement: at most 100 ms.
- Default continuation page: at most 100 branches and never larger than the
  request node bound.
- No excluded frontier branch may become unreachable.
- Structural output renders before optional Program IR enrichment.
- `compass call-graph --format json` remains one compatible JSON document.
- `compass.call_graph/1` receives additive fields only.
- `compass.program.call_graph/1` remains compatible in this PR.
- Root lookup, traversal, evidence merging, bounds, and cursor validation stay
  in Rust.
- Cache corruption, cache mismatch, and cache-write failure are recoverable.
- A missing or invalid Program IR artifact cannot remove a structural result.
- Superseded or disposed requests cannot render late progress or results.
- Keep the unrelated `editors/vscode/package.json` version change out of every
  commit.

## Stable implementation interfaces

Use these names throughout the six commits.

### Pagination and timing

```rust
pub const DEFAULT_CONTINUATION_PAGE_SIZE: usize = 100;

#[derive(Clone, Debug, Default)]
pub struct ContinuationPageRequest {
    pub cursor: Option<String>,
    pub page_size: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinuationPage {
    pub returned: usize,
    pub omitted: usize,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallGraphTimings {
    pub index_load: u64,
    pub root_resolution: u64,
    pub traversal: u64,
    pub enrichment: u64,
    pub resolver_total: u64,
}
```

`UniversalCallGraphResponse` gains:

```rust
pub artifact_fingerprint: String,
pub continuation_page: ContinuationPage,
pub timings_ms: CallGraphTimings,
```

### Shared traversal

```rust
pub(crate) struct TraversalEdge {
    pub source: String,
    pub target: String,
    pub continuable: bool,
    pub order_key: String,
}

pub(crate) struct TraversalInput<'a> {
    pub root: &'a str,
    pub direction: CallGraphDirection,
    pub depth: u32,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub incoming: &'a BTreeMap<String, Vec<usize>>,
    pub outgoing: &'a BTreeMap<String, Vec<usize>>,
    pub edges: &'a [TraversalEdge],
    pub cancellation: &'a AtomicBool,
}

pub(crate) struct TraversalSelection {
    pub node_ids: BTreeSet<String>,
    pub edge_indexes: Vec<usize>,
    pub frontier: Vec<TraversalFrontier>,
    pub truncated: bool,
}

pub(crate) fn traverse_calls(
    input: TraversalInput<'_>,
) -> Result<TraversalSelection, TraversalError>;
```

### Index loading

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactSignature {
    pub bytes: u64,
    pub modified_ns: u128,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheDisposition {
    Hit,
    Rebuilt,
    MemoryOnly,
}

pub struct LoadedCallIndex<T> {
    pub index: T,
    pub disposition: CacheDisposition,
    pub persistence_warning: Option<String>,
}
```

### VS Code request identity

```ts
export type CallGraphRequestIdentity = {
  repositoryId: string;
  artifactFingerprint: string;
  root: readonly string[];
  direction: CallDirection;
  depth: number;
  maxNodes: number;
  maxEdges: number;
  continuationCursor: string | null;
  evidenceLayer: "structural_graph" | "combined";
};
```

## Commit 1 — Paginate truncated continuations

**Commit message:** `feat(call-graph): paginate truncated continuations`

### Production changes

Create `crates/compass-analysis/src/continuation_cursor.rs`.

Define a private, canonical cursor payload:

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorPayload {
    version: u8,
    artifact_fingerprint: String,
    root_key: String,
    direction: CallGraphDirection,
    depth: u32,
    max_nodes: usize,
    max_edges: usize,
    evidence_layer: String,
    after_order_key: String,
}
```

Serialize with `serde_json::to_vec` and encode with base64url without padding.
Decode and validate every field before traversal. Return distinct
`InvalidCursor`, `StaleCursor`, and `CursorRequestMismatch` errors.

Update `UniversalCallGraphRequest`:

```rust
pub artifact_fingerprint: String,
pub continuation_page: ContinuationPageRequest,
```

Change structural traversal response shaping so it:

1. retains the complete ordered frontier metadata;
2. resumes strictly after the decoded `after_order_key`;
3. returns at most `min(page_size, max_nodes, 100)` continuations;
4. reports the exact remaining count;
5. emits a next cursor only when another page exists; and
6. keeps `truncated` true whenever node, edge, or depth bounds excluded work.

The initial CLI implementation may hash `graph.json` once to produce
`sha256:<hex>`. Commit 6 replaces that request-time artifact read with the
fingerprint stored in the compact index.

Extend `crates/compass-cli/src/call_graph_commands.rs` with:

```text
--continuation-cursor <OPAQUE_TOKEN>
```

Keep the current root, direction, depth, and limits in page requests so Rust
can compare them with the cursor identity.

Extend the additive Zod response contract in
`packages/compass-viewer/src/contracts/callGraph.ts`. Older responses without
the new fields remain valid.

Add:

```ts
export function mergeContinuationPage(
  current: CallGraphResponse,
  page: CallGraphResponse
): CallGraphResponse;
```

The viewer retains its existing twenty-button preview and “show all on this
page” action. It additionally renders the exact omitted count and a “Load more
branches” action when `nextCursor` exists. Symbol-specific branch expansion
remains unchanged.

Wire `loadContinuationPage` through:

- `packages/compass-viewer/src/calls/CallGraph.tsx`
- `editors/vscode/src/webviews/callGraph.tsx`
- `editors/vscode/src/views/callGraphArguments.ts`
- `editors/vscode/src/views/callGraphPanel.ts`

A stale cursor causes a page-one refresh, not a permanent panel error.

### Validation after implementation

Add coverage proving:

- all continuation pages are disjoint and collectively complete;
- deterministic replay produces identical pages;
- tampered, request-mismatched, and stale-artifact cursors fail explicitly;
- JSON without additive fields remains accepted;
- the viewer appends pages without duplicating branch buttons; and
- a continuation request retains the root identity and traversal limits.

Run:

```bash
cargo test -p compass-analysis --test universal_call_graph
cargo test -p compass-cli --test call_graph_cli
npm test -w @compass/viewer -- --run src/calls/state.test.ts src/calls/CallGraph.test.tsx
npm test -w editors/vscode -- --run src/views/callGraphArguments.test.ts
npm run typecheck:js
git diff --check
```

## Commit 2 — Share bounded traversal with Program IR

**Commit message:** `perf(call-graph): share bounded Program IR traversal`

### Production changes

Create `crates/compass-analysis/src/call_traversal.rs`.

The module accepts already indexed edges and adjacency. It does not own
response types, source anchors, evidence, or root lookup. Its algorithm:

1. starts with the root admitted;
2. reads only the relevant incoming/outgoing adjacency lists;
3. sorts and deduplicates edge indexes before applying bounds;
4. stops node admission at `max_nodes`;
5. stops edge admission at `max_edges`;
6. records excluded continuable neighbors in the frontier;
7. handles cycles and self-edges without duplicate queue work; and
8. checks cancellation every 256 records and whenever 25 ms has elapsed.

Expose `TraversalError::Cancelled`.

Adapt `universal_call_graph.rs` to project its structural edges into
`TraversalEdge`, call `traverse_calls`, and shape the selected indexes into the
existing universal response.

Adapt `call_graph.rs` to use the same engine. Preserve
`compass.program.call_graph/1` and its current response model while replacing
the `all_edges.iter().filter(...)` traversal loop.

Add:

```rust
pub fn build_call_graph_with_cancellation(
    analysis: &AnalysisBundle,
    graph: Option<&GraphDocument>,
    request: &CallGraphRequest,
    cancellation: &AtomicBool,
) -> Result<CallGraphResponse, CallGraphError>;
```

Keep `build_call_graph` as the compatible convenience wrapper. This shared
engine improves the current Program IR call graph without limiting its future
semantic model.

### Validation after implementation

Cover callers, callees, both, depth limits, node limits, edge limits, cycles,
self-edges, input permutations, high fanout, and pre-cancelled traversal.
Compare structural and Program IR adapter selection for the same topology.

Remove the strict debug-build wall-clock assertion from the 16,000-edge unit
fixture. Keep deterministic bounds and response-digest checks there; release
latency becomes Commit 3’s responsibility.

Run:

```bash
cargo test -p compass-analysis
cargo test -p compass-cli --test call_graph_cli
cargo clippy -p compass-analysis --all-targets -- -D warnings
rg "all_edges\\.iter\\(\\)\\.filter" crates/compass-analysis/src
git diff --check
```

The `rg` command must find no per-visited-node traversal loop.

## Commit 3 — Establish the release performance gate

**Commit message:** `test(call-graph): add performance qualification`

### Production changes

Create `scripts/benchmark_call_graph.sh` as the release measurement driver and
`scripts/check_call_graph_benchmark.py` as the deterministic result validator.

The driver accepts:

```text
--compass PATH
--repo PATH
--out PATH
--samples N
--baseline PATH
--no-enforce
```

It records:

- Compass version and commit;
- OS, architecture, CPU, memory, and Rust toolchain;
- graph and Program IR sizes and fingerprints;
- cache-miss migration;
- indexed cold-process structural latency;
- repeated session-cache latency once Commit 6 supplies that benchmark;
- cached Program IR enrichment;
- continuation-page latency and reachability;
- first-progress-event latency;
- cancellation acknowledgement;
- peak RSS;
- output bytes; and
- deterministic response digests.

Write raw CSV samples plus:

```text
target/call-graph-benchmark/results.json
```

The Python validator enforces:

```text
indexedColdMedian <= 500 ms
sessionWarmMedian <= 100 ms
enrichmentMedian <= 1000 ms
firstProgressMedian <= 100 ms
cancellationMedian <= 100 ms
median regression <= 10 percent
```

It also fails on digest drift, response-bound violations, inaccessible
continuations, or invalid terminal-event cardinality. During this commit,
metrics depending on later commits are recorded as `not_available`, and CI
uses `--no-enforce`.

Update:

- `.github/workflows/compass-ci.yml` to syntax-check the shell driver and run
  the validator’s standard-library self-test;
- `.github/workflows/compass-hardening.yml` to run the release measurement on
  schedule/manual dispatch and upload raw artifacts even after failure; and
- `PERFORMANCE.md` with exact reproduction and baseline-approval commands.

### Validation after implementation

Run:

```bash
bash -n scripts/benchmark_call_graph.sh
python3 scripts/check_call_graph_benchmark.py --self-test
cargo build --release -p compass-cli
scripts/benchmark_call_graph.sh \
  --compass target/release/compass \
  --repo . \
  --out target/call-graph-benchmark \
  --samples 1 \
  --no-enforce
git diff --check
```

Inspect `results.json` and confirm every available metric includes its raw
samples and deterministic digest.

## Commit 4 — Stream real resolution progress

**Commit message:** `feat(call-graph): stream resolver progress`

### Production changes

Create `crates/compass-cli/src/call_graph_events.rs` with schema:

```rust
pub const CALL_GRAPH_EVENT_SCHEMA: &str = "compass.call_graph.events/1";
```

Support:

```text
compass call-graph ... --format jsonl
```

Keep `--format json` unchanged.

The JSONL stream contains:

```json
{"type":"progress","phase":"loading_structural_index","elapsedMs":4,"terminal":false}
{"type":"progress","phase":"locating_symbol","elapsedMs":7,"terminal":false}
{"type":"progress","phase":"tracing_calls","elapsedMs":11,"terminal":false}
{"type":"progress","phase":"serializing_result","elapsedMs":12,"terminal":false}
{"type":"result","graph":{},"timingsMs":{},"terminal":true}
```

Structural phases:

```text
loading_structural_index
locating_symbol
tracing_calls
serializing_result
```

Enrichment phases:

```text
loading_program_index
joining_evidence
serializing_result
```

Add a writer that flushes every record and guarantees exactly one terminal
result or error on normal completion. Host-requested cancellation may terminate
without a terminal record.

Add `run_call_graph_jsonl` to `crates/compass-cli/src/lib.rs` and route it
directly from `src/bin/compass.rs` before the buffered `Outcome` path. Install
the existing `process_cancellation()` atomic and pass it into index/traversal
work.

Measure actual boundaries with `Instant`. Put index load, root resolution,
traversal, enrichment, and resolver-total values in the response. Put
serialization and end-to-end values in the terminal stream envelope because
they are not known until encoding completes.

Advertise:

```json
"contracts": {
  "call_graph_events": "compass.call_graph.events/1"
},
"features": {
  "call_graph_events": true
}
```

Create `editors/vscode/src/cli/callGraphEvents.ts` and add a generic bounded
runner to `CompassProcessManager`:

```ts
async runJsonl<E, R>(
  cwd: string,
  args: readonly string[],
  schema: ZodType<E>,
  terminalResult: (event: E) => R | undefined,
  onEvent: (event: E) => void,
  signal?: AbortSignal
): Promise<R>;
```

Retain `startJsonl` for existing IDE progress users.

Map resolver phases to the existing loading steps and post progress only when
the request generation is current. Log requests above 500 ms with fingerprint,
artifact bytes, direction, depth, result counts, cache disposition, and phase
timings.

### Validation after implementation

Cover split JSONL chunks, malformed events, duplicate terminals, failure
terminals, output above 8 MiB, cancellation, real phase order, and late event
suppression. Confirm the first event is flushed rather than buffered.

Run:

```bash
cargo test -p compass-cli --test call_graph_events_cli
cargo test -p compass-cli --test capabilities_cli
npm test -w editors/vscode -- --run src/cli/processManager.test.ts src/views/callGraphArguments.test.ts
npm run typecheck:js
git diff --check
```

## Commit 5 — Add cached progressive Program IR enrichment

**Commit message:** `perf(call-graph): cache progressive Program IR enrichment`

### Production changes

Create `crates/compass-analysis/src/call_index_cache.rs` for the focused cache
envelope and atomic MessagePack persistence. Do not create a general cache
framework.

The header contains:

```text
cache magic
cache schema
source artifact byte length
source modification timestamp
source artifact SHA-256
```

Cache reads validate the complete header before accepting the payload. A newer
schema, malformed payload, stale signature, or digest mismatch is a cache miss.
Writes use a uniquely named sibling temporary file, file sync, atomic rename,
and best-effort directory sync.

Create `crates/compass-analysis/src/program_call_index.rs` with:

```rust
pub const PROGRAM_CALL_INDEX_SCHEMA: &str = "compass.program_call_index/1";
```

The projection stores only:

- functions keyed by Program symbol and `graph_node_id`;
- source byte anchors;
- resolved, inferred, ambiguous, and unresolved calls;
- exact call-site anchors and evidence IDs; and
- sorted incoming/outgoing adjacency.

On the first cache miss, call the existing Program loader so size, schema,
semantic validation, and canonical-byte verification still occur before
projection. Subsequent matching requests deserialize the compact index instead
of the full canonical Program IR.

Update universal enrichment to accept `ProgramCallIndex`. It preserves
structural IDs and relationships, adds exact anchors/evidence, adds
Program-only unresolved or ambiguous calls, deduplicates call sites, and sets
the evidence layer to `combined`.

In VS Code:

1. render the structural terminal result immediately;
2. check whether `session.programPath` exists;
3. start a separate cancellable `--program` JSONL request;
4. show a compact “Adding semantic evidence…” status over the visible graph;
5. merge only when repository, generation, root, direction, depth,
   fingerprint, and response schema still match; and
6. keep the structural graph visible after enrichment failure.

Use separate structural and enrichment controllers. Root/direction changes and
panel disposal abort both. Retry targets enrichment only.

Add `mergeEnrichment` in `packages/compass-viewer/src/calls/state.ts`. It
deduplicates call sites, retains structural data, adds semantic data, and
recalculates coverage.

### Validation after implementation

Validate Program index deterministic bytes, valid round trip, corruption,
schema mismatch, stale signatures, interrupted writes, read-only persistence,
and concurrent readers.

Validate UI ordering and safety:

- structural hydrate precedes enrichment status and merge;
- missing Program IR completes structurally;
- invalid Program IR is nonfatal;
- enrichment retry does not reload the structural graph;
- stale generations cannot merge; and
- panel disposal aborts both processes.

Run:

```bash
cargo test -p compass-analysis --test call_index_cache
cargo test -p compass-cli --test call_graph_cli
npm test -w @compass/viewer -- --run src/calls/state.test.ts src/calls/CallGraph.test.tsx
npm test -w editors/vscode -- --run src/views/callGraphPanel.test.ts src/views/callGraphArguments.test.ts
npm run typecheck:js
git diff --check
```

## Commit 6 — Persist compact structural indexes and add the session cache

**Commit message:** `perf(call-graph): persist compact call indexes`

### Production changes

Create `crates/compass-analysis/src/structural_call_index.rs` with:

```rust
pub const STRUCTURAL_CALL_INDEX_SCHEMA: &str = "compass.call_index/1";

pub struct StructuralCallIndex {
    pub schema: String,
    pub artifact_fingerprint: String,
    pub nodes: BTreeMap<String, IndexedCallNode>,
    pub file_nodes: BTreeMap<String, Vec<String>>,
    pub edges: Vec<IndexedStructuralEdge>,
    pub incoming: BTreeMap<String, Vec<usize>>,
    pub outgoing: BTreeMap<String, Vec<usize>>,
}
```

The projection contains only callable nodes/ranges, normalized file lookup,
structural call edges, evidence metadata, deterministic ordering keys, and
incoming/outgoing adjacency.

Normalize and sort source lookup during index construction:

```text
smallest containing range
callable-kind rank
node ID
```

Sort structural edges before building adjacency. Check cancellation every 256
records and at least every 25 ms.

Expose:

```rust
pub fn load_or_build_structural_call_index(
    graph_path: &Path,
    verified_sha256: Option<&str>,
    cancellation: &AtomicBool,
) -> Result<LoadedCallIndex<StructuralCallIndex>, CallIndexError>;
```

On a cache hit, do not deserialize `graph.json`. On a miss, validate the graph,
build the projection, compute its SHA-256, attempt atomic persistence, and
continue from the in-memory result if the write fails.

Update `crates/compass-core/src/build_state.rs` with read-only graph/program
seal accessors. Update `crates/compass-core/src/pipeline.rs` to publish both
indexes after valid canonical artifacts exist. Use the verified seals when
available. Index publication failure emits a diagnostic but cannot fail the
update. An unchanged warm update also creates a missing index.

Create `editors/vscode/src/views/callGraphCache.ts`:

```ts
export class CallGraphResponseCache {
  constructor(options?: {
    maxEntriesPerRepository: number;
    maxBytes: number;
  });

  get(identity: CallGraphRequestIdentity): CallGraphResponse | undefined;
  set(identity: CallGraphRequestIdentity, response: CallGraphResponse): void;
  invalidateRepositoryFingerprint(
    repositoryId: string,
    currentFingerprint: string
  ): void;
}
```

Defaults:

```text
8 entries per repository
16 MiB estimated serialized total
```

Estimate weight with
`Buffer.byteLength(JSON.stringify(response), "utf8")`. Store only
Zod-validated response objects. Evict in least-recently-used order. A new graph
fingerprint invalidates older entries for that repository. Structural and
combined evidence, cursor pages, directions, depths, roots, and limits have
distinct keys.

The panel checks the cache before spawning a CLI process. A structural cache
hit renders immediately and may still start Program enrichment. A matching
combined cache hit finishes without spawning either request.

Finally, enable every benchmark assertion, add the session-cache measurement,
remove `--no-enforce` from the hardening workflow, and retain the raw result
artifacts.

### Validation after implementation

Validate:

- indexed and direct graph resolution are identical;
- index bytes are deterministic;
- corrupt, newer, stale, and interrupted caches rebuild safely;
- a read-only cache still returns a graph;
- `compass update` publishes both indexes;
- index publication failure leaves canonical artifacts valid;
- LRU hit, recency, entry-count eviction, byte eviction, repository isolation,
  cursor separation, evidence separation, and fingerprint invalidation; and
- all five latency budgets on the retained release fixture.

Run the complete branch gate:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p compass-cli
npm run test:js
npm run typecheck:js
npm run build
scripts/benchmark_call_graph.sh \
  --compass target/release/compass \
  --repo . \
  --out target/call-graph-benchmark \
  --samples 7
git diff --check
```

After code changes, refresh the outer knowledge graph:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

Inspect the final commit list and ensure the unrelated extension version change
is not staged:

```bash
cd /Users/haipingfu/graphify/compass
git status --short
git log --oneline origin/main..HEAD
git diff --check
```

Push the branch and update draft PR #74 with:

- the six production changes;
- compatibility and failure behavior;
- release fixture and measured metrics;
- verification commands;
- benchmark artifact location; and
- confirmation that Program IR call graphs remain a first-class enhancement
  surface.

Do not mark the PR ready until required GitHub checks and the hardening job are
green.

## Final implementation acceptance

- Every omitted continuation is reachable exactly once through pagination.
- Structural and Program IR call graphs both use `traverse_calls`.
- Neither traversal scans the complete edge set for each visited node.
- Structural output appears before Program IR enrichment.
- Program IR enrichment adds semantic evidence without becoming a structural
  prerequisite.
- Real resolver phases drive the loading UI.
- Cancellation and generation checks prevent stale rendering.
- Structural and Program index failures degrade to valid in-memory behavior.
- Indexed structural resolution, session hits, enrichment, progress, and
  cancellation meet their approved budgets.
- Both first-class call-graph contracts remain compatible within this PR.
- PR #74 contains the approved six production commits in the specified order.
