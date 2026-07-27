# Versioned History Performance

**Date:** 2026-07-26
**Status:** Approved for implementation planning — hard cutover

## Summary

Compass current-tree indexing is fast because it retains portable, content-addressed
AST and Program caches. Exact-revision history currently loses those advantages:
temporary worktrees own temporary caches, ordinary reads repeatedly perform full
realization validation and reconstruction, and semantic diff performs many
independent evidence lookups.

This design makes first access to an unseen commit a bounded extraction while
making repeated builds, semantic diffs, and historical graph views effectively
instant. It preserves immutable realization identity, exact-commit isolation,
deterministic reports, and fail-closed handling of authoritative corruption.

## Measured Baseline

The current debug binary was measured against two existing CocoIndex
realizations:

- base `90571539fa291fc6e6b248095bd2c8a2ff68bab4`;
- target `71f9cc9dc693080310181a2d011fb737420f7907`;
- 17,729–17,808 nodes;
- 40,237–40,412 edges;
- 91,163–91,537 Program facts;
- 10,931–10,981 Program summaries.

Observed results:

| Operation | Wall time | Maximum RSS |
|---|---:|---:|
| Semantic diff | 5.19 s | 203 MiB |
| Historical viewer export | 152.40 s | 1.78 GiB |
| Existing build with `--profile-from` | 154.65 s | 1.77 GiB |

The latter two commands ran concurrently and therefore include contention, but
profiles and code inspection confirm the dominant behavior: both scan and
reconstruct complete realization trees. Earlier Podman qualification measured a
full history status/profile-validation path at approximately 189 seconds and
4.7 GiB RSS for 119,293 nodes and 262,724 edges.

## Goals

1. Build an arbitrary exact Git commit using the same portable extraction
   acceleration available to current-tree indexing.
2. Make an existing matching realization a constant-sized manifest lookup,
   not a complete validation pass.
3. Compare two materialized realizations without reconstructing either graph.
4. Generate and reuse deterministic semantic-diff reports with bounded storage
   reads.
5. Open a historical graph overview immediately and load exact detail lazily.
6. Preserve current graph, Program IR, semantic report, and export semantics.

## Non-goals

- Guarantee sub-second extraction for a commit whose source content has never
  been indexed.
- Require full-history pre-indexing.
- Replace the protected detached-worktree security boundary.
- Move authoritative realization content out of the Prolly store.
- Treat derived caches as authoritative.
- Weaken publication validation or corruption recovery.
- Change `compass.semantic_diff.report/1` in this performance phase.
- Migrate, import, or continue reading legacy cache entries.
- Preserve superseded internal Rust APIs, cache constructors, CLI response
  fields, or mixed old/new execution paths.

## Hard Cutover

This performance architecture ships as one hard cutover:

- all Compass cache call sites move to the new explicit cache options API;
- the old `Cache::new` constructor, legacy directory lookup, legacy JSON
  decoding, prompt-fingerprint fallback, and legacy build-state validation are
  deleted;
- all history consumers move to `RealizationReader`; the old one-shot
  `HistoryStore::read_record` and store-owned diff APIs are deleted;
- `history status` emits only the new seal contract, without deprecated
  validation fields;
- old extraction and derived cache entries are ignored and may be removed by
  cache maintenance; no importer or on-read migration is implemented;
- the complete implementation lands before release, with no feature flag,
  compatibility mode, or dual-write period.

The immutable Prolly realization format is unchanged because this performance
work does not require a new authoritative representation. Reading an existing
realization with the same `HISTORY_SCHEMA_VERSION` is therefore the normal
current format, not a compatibility path. If implementation requires any
authoritative format change, it must bump the store/history schema and reject
the old format explicitly; it must not add a migration adapter.

## Latency and Memory Contract

Targets apply to release builds on local SSD storage.

| Operation | CocoIndex-sized (~20k nodes) | Podman-sized (~120k nodes) |
|---|---:|---:|
| Existing historical overview | ≤250 ms warm, ≤1 s without projection cache | ≤500 ms warm, ≤2 s without projection cache |
| Repeated semantic diff | ≤250 ms | ≤500 ms |
| First diff of materialized graphs | ≤1 s | ≤2 s |
| No-op historical build | ≤250 ms | ≤500 ms |
| Adjacent historical build | ≤2× current incremental update | ≤2× current incremental update |
| First unseen cold commit | ≤1.25× equivalent cold current-tree extraction | ≤1.25× equivalent cold current-tree extraction |

Diff and historical viewing must remain below 512 MiB peak RSS on the Podman
qualification repository. Historical build memory must remain within 25% of an
equivalent current-tree extraction.

These are acceptance targets rather than reasons to weaken correctness. A
measured miss is reported as a performance gap.

## Storage Planes

Authoritative and derived state remain visibly separate:

```text
<git-common-dir>/compass/
├── history.sqlite
├── cache/
│   └── v1/
│       ├── ast/
│       ├── program-syntax/
│       ├── program-artifact/
│       ├── program-merge/
│       ├── semantic-diff/
│       └── viewer/
├── jobs/
├── leases/
├── locks/
└── tmp/
```

`history.sqlite` remains authoritative. Everything under `cache/v1` is
rebuildable and may be removed without changing graph meaning.

All cache resources use owner-only paths. Writes are atomic and
content-addressed. Concurrent writers may compute the same entry, but publication
of identical bytes is idempotent.

## Shared Historical Extraction Cache

### Identity

AST entries are keyed by:

- source content digest;
- AST extractor version;
- cache encoding version;
- meaning-affecting parser configuration.

Program entries additionally include:

- Program IR schema;
- provider/analyzer/merger versions;
- provider input identity where applicable;
- normalized repository-relative logical identity.

The checkout root, temporary-worktree name, file modification time, and absolute
path are excluded from content identity. Loaded records are rebased to the exact
checkout only at the API boundary.

### Lookup and publication

Historical extraction receives an explicit repository-shared cache root rather
than deriving cache location from the temporary output directory. The builder:

1. resolves the exact commit;
2. creates the protected detached worktree;
3. detects sources under historical ignore rules;
4. loads portable AST and Program entries from the shared cache;
5. extracts only cache misses;
6. publishes new cache entries atomically;
7. assembles, validates, and publishes the immutable realization.

Temporary-worktree cleanup does not remove the shared cache.

### Adjacent realization reuse

The current implementation reconstructs every authoritative tree from a
matching-profile ancestor and writes a complete temporary `compass-out` seed.
The hard-cutover builder deletes ancestor artifact seeding entirely. Shared
content-addressed AST and Program entries identify reusable work. Historical
clustering is already deterministic and intentionally ignores prior
communities, so no compact legacy seed path is retained.

### Current-tree interoperability

Current-tree and history extraction move together to the new explicit cache API
and portable entry encoding. They use separate storage roots and do not import,
hard-link, or decode the prior cache format. Historical correctness never
depends on a mutable current-tree cache. Exact-HEAD promotion remains a current
performance path, not a legacy adapter.

## Sealed Realization Fast Path

Publication continues to:

1. validate completion evidence;
2. partition authoritative artifacts;
3. construct content-addressed trees;
4. validate counts, canonical records, references, and artifact reconstruction;
5. publish the immutable manifest and direct roots atomically;
6. update the preferred pointer by compare-and-swap.

The published manifest already contains the realization digest, exact commit,
profile, record counts, and direct tree roots. Ordinary trusted reads therefore
perform a bounded seal check:

- parse and validate the manifest;
- recompute the realization ID;
- verify direct named roots match the manifest;
- verify the requested commit/profile relationship.

They do not scan every record.

Full validation remains mandatory:

- before initial publication;
- for explicit integrity/maintenance commands;
- before corrupt-preferred recovery;
- when a manifest or direct root check fails;
- when an operation observes impossible decoded data.

Corruption in authoritative state fails closed. There is no automatic fallback
from a corrupt realization to an unverified result.

## Historical Build Flow

### Existing realization

`history build REV` and `--profile-from` read sealed manifests only. A matching
preferred realization returns without full validation, extraction, or
republication.

`--profile-from` copies the normalized profile from the manifest. It does not
load graph records.

### Unseen adjacent commit

The builder uses the shared cache directly. Source content already seen in any
worktree or branch under the new cache identity is reused. Only unseen or
version-invalidated entries are parsed and analyzed. It does not inspect or
reconstruct a first-parent realization.

### First unseen arbitrary commit

The exact commit is checked out and all source entries are looked up by content.
Any globally warm entries are reused. Remaining entries receive a bounded full
extraction. This operation is not required to be sub-second.

## Semantic Diff

### First comparison

The comparison path:

1. resolves both exact commits and sealed matching manifests;
2. obtains the exact Git source delta;
3. streams changed node and edge records through Prolly root comparison;
4. derives the minimal set of Program modules, functions, summaries, reverse
   callers, and graph nodes needed for findings;
5. batch-loads those keys under one activity guard and one set of opened roots;
6. computes and renders the deterministic semantic report.

`HistorySnapshots` becomes a request-scoped batch/memoized reader. A record is
decoded at most once per comparison. Manifest and direct root lookup are also
performed once per realization.

### Cached comparison

The semantic-diff cache key includes:

- old and new realization IDs in direction-sensitive order;
- source-delta digest;
- semantic-diff engine version;
- report schema;
- rendering-neutral comparison options.

The cache value is the canonical complete report, not a text or HTML
presentation. Text limits, `--all`, `--explain`, JSON, and HTML are rendered from
the same cached report.

Cache hits verify the key and payload digest. Invalid entries are removed or
ignored and recomputed. Cache failure cannot invalidate either realization.

## Historical Graph Viewing

Each new realization publishes or schedules a derived viewer projection from the
already in-memory completed graph. It contains:

- bounded overview nodes and edges;
- community summaries and labels;
- graph statistics;
- stable identities needed for lazy detail;
- projection schema and source realization ID.

The initial viewer request loads only this projection. Exact node, neighborhood,
and community detail is read lazily from typed node/edge records. Program facts,
semantic metadata, and authoritative sidecars are not loaded for an overview.

Any projection miss streams the required graph roots once, writes a derived
projection, and serves the result. This is the sole viewer miss path; there is
no separate older-realization upgrade or migration branch.

Viewer cache corruption causes regeneration. Authoritative graph corruption
still fails closed.

## Cache Lifecycle

Cache GC is independent of immutable history GC. It supports:

- removal of obsolete extractor/schema namespaces;
- least-recently-used pruning to a configurable byte budget;
- age-based removal of semantic-diff and viewer entries;
- preservation of entries currently used by active builders/readers;
- dry-run reporting before destructive maintenance.

Normal history GC does not silently delete shared extraction entries that may
accelerate other branches. Cache GC never deletes Prolly realization content.

## Failure Handling

| Failure | Behavior |
|---|---|
| Missing cache entry | Compute and publish it |
| Malformed extraction cache | Treat as miss |
| Semantic-diff cache digest mismatch | Ignore and recompute |
| Viewer projection mismatch | Ignore and regenerate |
| Shared cache write race | Accept identical winner; retry or reuse |
| Shared cache unavailable | Fail the build with the exact cache path and cause |
| Manifest digest/root mismatch | Fail closed and recommend full validation |
| Full validation failure | Enter the current corrupt-recovery workflow |
| Temporary worktree cleanup failure | Report the cleanup error contract |

## Security

The detached-worktree policy remains unchanged: no fetch, prompts, user hooks,
checkout filters, LFS smudge, or submodule recursion.

Shared cache entries are data, never executable code. Decoders retain byte,
record-count, and nesting limits. Repository-relative paths are validated before
rebasing. Cache keys never contain credentials. Semantic provider credentials
remain outside build profiles and caches.

Derived report and viewer caches must not contain absolute temporary-worktree
paths.

## Testing Strategy

### Operation-count tests

Counting storage adapters and focused integration seams assert:

- no-op build does not invoke full-tree validation;
- profile lookup reads only the sealed manifest;
- ordinary diff never reconstructs a complete graph;
- historical overview does not open Program, metadata, or sidecar roots;
- a comparison opens each requested root once;
- each evidence record is decoded at most once;
- adjacent builds reuse unchanged portable cache entries across different
  temporary worktree paths.

### Correctness tests

- Cache hit and miss builds produce identical canonical graph and Program bytes.
- Cached and uncached semantic reports have identical SHA-256 digests.
- Source patches and stable finding IDs are unchanged.
- A projection miss uses the same graph-only regeneration path regardless of
  realization age.
- Corrupt derived entries regenerate.
- Corrupt authoritative manifests or roots fail closed.
- Parallel builders cannot publish mixed cache bytes or incomplete realizations.
- Legacy cache files, legacy build state, old cache constructors, and old
  history reader APIs are absent after cutover.

### Real-repository qualification

Release-mode benchmarks run on CocoIndex and Podman and capture:

- wall time;
- peak RSS;
- cache hits and misses by class;
- records and Prolly roots opened;
- validation/reconstruction calls;
- output digests.

Each benchmark records current-tree cold and incremental extraction so historical
build ratios use a same-machine, same-revision baseline.

## Delivery

Implementation uses reviewable commits:

1. sealed-manifest fast paths and explicit full-validation boundaries;
2. shared portable history extraction cache;
3. batched/memoized semantic evidence reads;
4. canonical semantic-report cache;
5. historical viewer projections and lazy detail;
6. cache lifecycle and real-repository release qualification.

All commits merge and release together. Intermediate commits are not supported
deployment states. There is no feature flag, dual read, dual write, cache
import, or compatibility window. The release starts with an empty new cache
namespace and repopulates it on demand. Existing immutable realizations remain
current-format data because the authoritative schema is unchanged.

## Acceptance Criteria

The work is complete when:

1. every latency and memory target has a recorded release-mode result or an
   explicitly documented shortfall;
2. ordinary no-op build, diff, and view paths do not call full reconstruction;
3. adjacent commits reuse content across temporary worktrees;
4. cached and uncached authoritative outputs are byte- or canonically identical;
5. the history, semantic-diff, security, concurrency, corruption, and explicit
   hard-cutover suites pass;
6. CocoIndex and Podman checkouts remain unchanged after qualification.
7. no legacy cache decoder, directory fallback, compatibility constructor,
   one-shot history reader, dual CLI field, or migration branch remains.
