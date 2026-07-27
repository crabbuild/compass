# Versioned History Performance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. This plan
> intentionally follows the user-approved implementation-first order:
> implement each production slice, add its regression and performance coverage,
> verify it, and then commit it. Do not reorder the work into a red/green TDD
> sequence.

**Goal:** Make arbitrary-revision graph materialization reuse Compass's portable
content cache, make existing realization reads bounded by sealed manifests, make
semantic diff reuse batched evidence and canonical reports, and make historical
graph views load from bounded projections.

**Architecture:** Keep `history.sqlite` authoritative and immutable. Add a
repository-private `cache/v1` plane below the Git common directory for portable
AST/Program entries and checked derived payloads. Ordinary reads trust the
already-published manifest seal and direct roots; explicit verification and
publication retain complete validation. Semantic diff uses two request-scoped
readers and a direction-sensitive canonical-report cache. Historical viewer
exports scan only graph-relevant roots on a miss and use a cached projection on
subsequent requests.

**Tech Stack:** Rust 2024, Compass's existing Prolly/SQLite history store,
portable MessagePack extraction cache, canonical JSON, Git detached worktrees,
shell/Python qualification tooling, existing VS Code Compass CLI integration.

**Approved design:**
`docs/superpowers/specs/2026-07-26-versioned-history-performance-design.md`

---

## Global constraints

- The first request for an unseen arbitrary commit may perform one bounded exact
  extraction in the protected detached worktree.
- A matching, already-published realization must not perform full-tree
  validation, artifact reconstruction, or extraction.
- `history.sqlite` remains the only authoritative realization store. Every file
  below `cache/v1` is disposable and reproducible.
- Publication, explicit integrity verification, preferred-pointer repair, and
  corrupt-recovery paths retain full validation.
- Manifest or direct-root corruption fails closed. Malformed derived or
  extraction cache entries are cache misses.
- Do not fetch, run hooks, enable smudge filters, recurse submodules, or weaken
  the current detached-worktree boundary.
- Cache identity must not contain a temporary-worktree path or depend on its
  file modification times.
- Preserve deterministic realization IDs, graph bytes, Program bytes, semantic
  report IDs, source patches, and report ordering.
- Preserve the current `compass.semantic_diff.report/1` and
  `compass.history.viewer_graph/1` public schemas.
- Ship a hard cutover: no legacy cache reads, cache imports, on-read migrations,
  deprecated Rust APIs, dual CLI fields, feature flags, or mixed old/new
  execution paths.
- Delete `Cache::new`, `legacy_directory`, legacy JSON/MessagePack fallback,
  `allow_legacy`, legacy build-state validation, `HistoryStore::read_record`,
  and store-owned diff entry points after migrating every workspace call site.
- Start the new cache namespace empty. Ignore prior cache files; cache GC may
  remove them.
- Keep `HISTORY_SCHEMA_VERSION` unchanged because the authoritative Prolly
  representation does not change. If implementation requires an authoritative
  format change, bump the schema and reject old stores explicitly; do not write
  a migration adapter.
- Keep the VS Code extension on Compass CLI commands; do not introduce a
  Graphify runtime dependency.
- Preserve all unrelated worktree changes. Stage only files named by the active
  task.
- After production code changes, run `graphify update .` from the Compass
  repository as required by `AGENTS.md`.

## Performance contract

Release-mode qualification on local SSD storage must target:

| Operation | CocoIndex-sized graph | Podman-sized graph |
|---|---:|---:|
| Existing overview, cached | ≤250 ms | ≤500 ms |
| Existing overview, projection miss | ≤1 s | ≤2 s |
| Repeated semantic diff | ≤250 ms | ≤500 ms |
| First diff of materialized graphs | ≤1 s | ≤2 s |
| No-op history build | ≤250 ms | ≤500 ms |
| Adjacent history build | ≤2× equivalent current incremental update | ≤2× equivalent current incremental update |
| First unseen commit | ≤1.25× equivalent current cold extraction | ≤1.25× equivalent current cold extraction |

Podman semantic diff and historical overview must remain below 512 MiB peak
RSS. Historical build RSS must remain within 25% of the equivalent current-tree
extraction.

The measured pre-change CocoIndex baseline is:

- semantic diff: 5.19 seconds, 203 MiB;
- viewer export: 152.40 seconds, 1.78 GiB;
- existing `history build --profile-from`: 154.65 seconds, 1.77 GiB.

Record an acceptance miss as a performance gap. Do not weaken validation,
semantic correctness, or deterministic output to hide it.

---

## Stable storage layout

All work in this plan uses this repository-private layout:

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

The `CacheKind` version subdirectories are recreated below these roots using
only the new encoding. Do not copy, import, hard-link, or decode entries from
the previous cache layout. Do not copy the current-tree `stat-index.json` into
shared history storage.

## Stable production interfaces

Use the following names and preserve these boundaries and ownership rules.

### Shared and derived cache

Add `crates/compass-history/src/cache.rs` with these public concepts:

```rust
pub const HISTORY_CACHE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivedCacheNamespace {
    SemanticDiff,
    Viewer,
}

#[derive(Clone, Debug)]
pub struct HistoryCache {
    root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CacheStatus {
    pub files: u64,
    pub bytes: u64,
    pub namespaces: BTreeMap<String, CacheNamespaceStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CacheGcPlan {
    pub files: u64,
    pub bytes: u64,
    pub paths: Vec<PathBuf>,
}
```

`HistoryStore::cache()` returns a validated owner-only `HistoryCache`.
`HistoryCache::extraction_root()` returns the exact `cache/v1` directory passed
to `compass-files`. Derived reads and writes take a namespace plus canonical key
material:

```rust
pub fn read(
    &self,
    namespace: DerivedCacheNamespace,
    key_material: &Value,
    max_payload_bytes: u64,
) -> Result<Option<Vec<u8>>, HistoryError>;

pub fn write(
    &self,
    namespace: DerivedCacheNamespace,
    key_material: &Value,
    payload: &[u8],
) -> Result<(), HistoryError>;
```

The on-disk envelope stores:

```json
{
  "schema": "compass.history.cache_entry/1",
  "namespace": "semantic-diff",
  "key_sha256": "<lowercase sha256>",
  "payload_sha256": "<lowercase sha256>",
  "payload": {}
}
```

`payload` is the semantic report or viewer envelope as a JSON value. The
payload digest is computed over its canonical JSON bytes. A hit must verify the
requested key, payload length, and payload digest before returning those
canonical bytes. A malformed envelope is ignored as a miss. Writes use the
existing atomic file helpers and owner-only directories.

The materializer already holds the shared history activity lock while its child
extractor runs. Derived readers also hold an activity guard. Cache GC acquires
the exclusive maintenance guard, so it cannot remove entries used by a live
builder or reader.

### Portable extraction cache mode

Replace the cache constructor/API in `crates/compass-files/src/cache.rs` with an
explicit layout/hash policy:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheLayout {
    OutputDirectory,
    SharedHistory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheHashPolicy {
    StatIndexed,
    VerifiedContent,
}

pub struct CacheOptions<'a> {
    pub storage_root: Option<&'a Path>,
    pub layout: CacheLayout,
    pub hash_policy: CacheHashPolicy,
}
```

Expose one constructor:

```rust
pub fn open(root: impl AsRef<Path>, options: CacheOptions<'_>)
    -> Result<Self, FileError>;
```

Migrate every workspace caller to `Cache::open` and delete `Cache::new`.
`CacheLayout::OutputDirectory` creates the current-tree layout under the
selected output root. `CacheLayout::SharedHistory`:

- writes directly below `cache/v1`, without inserting
  `<output-name>/cache`;
- computes the key from normalized repository-relative path plus file bytes,
  extractor version, and existing cache-kind versions;
- always reads the file bytes instead of trusting a persisted size/mtime index;
- does not flush `StatHashIndex`;
- retains repository-relative Program and AST values.

Delete `legacy_directory`, the `allow_legacy` argument, legacy JSON decode
branches, legacy Program fallback, and pruning of legacy paths. Bump the
deterministic cache encoding namespace so old entries cannot be selected by the
new code.

The relative logical path stays in the identity because extracted graph and
Program facts contain `source_file`. The absolute checkout root and mtime do
not.

### Request-scoped realization reader

Add `crates/compass-history/src/reader.rs`:

```rust
pub struct RealizationReader<'store> {
    store: &'store HistoryStore,
    activity: ActivityGuard,
    published: PublishedVersion,
    // Lazily opened roots and decoded-record memoization are private.
}

impl HistoryStore {
    pub fn reader(
        &self,
        realization: &RealizationId,
    ) -> Result<RealizationReader<'_>, HistoryError>;
}

impl RealizationReader<'_> {
    pub fn version(&self) -> &PublishedVersion;

    pub fn read(
        &self,
        key: HistoryRecordKey<'_>,
    ) -> Result<Option<HistoryRecord>, HistoryError>;

    pub fn read_many<'key>(
        &self,
        keys: impl IntoIterator<Item = HistoryRecordKey<'key>>,
    ) -> Result<Vec<Option<HistoryRecord>>, HistoryError>;

    pub fn diff(
        &self,
        new: &RealizationReader<'_>,
        sink: &mut dyn ChangeSink,
    ) -> Result<(), HistoryError>;

    pub fn diff_records(
        &self,
        new: &RealizationReader<'_>,
        records: &[RecordKind],
        sink: &mut dyn ChangeSink,
    ) -> Result<(), HistoryError>;
}
```

The reader:

- opens and seal-checks the realization once;
- retains one activity guard;
- lazily opens each requested root at most once;
- memoizes positive and negative record lookups by an owned internal key;
- decodes each requested record at most once;
- retains the existing value-size and typed-schema checks.

Migrate all callers in the same cutover and delete
`HistoryStore::read_record`, `HistoryStore::diff`, and
`HistoryStore::diff_records`. There is no one-shot wrapper.

### Graph-only scan

Add `crates/compass-history/src/graph_read.rs`:

```rust
pub trait GraphRecordSink {
    fn node_attribute(
        &mut self,
        node_id: String,
        field: String,
        value: Value,
    ) -> Result<(), HistoryError>;

    fn labels(&mut self, labels: Value) -> Result<(), HistoryError>;
    fn node(&mut self, node: NodeRecord) -> Result<(), HistoryError>;
    fn edge(&mut self, edge: EdgeRecord) -> Result<(), HistoryError>;
}

impl RealizationReader<'_> {
    pub fn scan_graph(
        &self,
        sink: &mut dyn GraphRecordSink,
    ) -> Result<(), HistoryError>;
}
```

`scan_graph` reads only `analysis`, `nodes`, and `edges`, in that order. It
decodes node analysis attributes and `.compass_labels.json`, ignores unrelated
analysis sidecars, and never opens hyperedges, metadata, Program facts, Program
summaries, or authoritative sidecars. Counts and record byte limits remain
enforced.

Add `crates/compass-output/src/history_viewer.rs` with a
`HistoricalViewBuilder` implementation of `GraphRecordSink`. It produces the
existing `GraphViewModel` directly:

- for a graph at or below `node_limit`, retain exact nodes and edges;
- for a larger graph, retain `node_id -> community`, community counts, labels,
  and aggregated inter-community edges only;
- for `--community ID`, retain only that community's exact nodes and internal
  edges;
- preserve the existing colors, labels, source locations, degree fields,
  aggregation flag, ordering, and viewer schema.

Extract reusable aggregation helpers from `compass-output/src/html.rs` instead
of maintaining two subtly different algorithms.

---

## File map

### New files

- `crates/compass-history/src/cache.rs`
  - Owner-only cache paths, checked derived envelopes, status, and GC planning.
- `crates/compass-history/src/reader.rs`
  - Sealed request-scoped readers, lazy roots, memoized typed lookups.
- `crates/compass-history/src/graph_read.rs`
  - Graph-only typed streaming contract.
- `crates/compass-output/src/history_viewer.rs`
  - Exact and aggregated viewer projection builder.
- `docs/superpowers/reviews/2026-07-26-versioned-history-performance-qualification.md`
  - Final CocoIndex and Podman release measurements and output digests.

### Modified production files

- `crates/compass-files/src/cache.rs`
  - Single new cache API, shared layout, verified-content hashing, and removal
    of all legacy readers.
- `crates/compass-files/src/lib.rs`
  - Export only the new cache options.
- `crates/compass-files/tests/contracts.rs`
  - Replace old-constructor/legacy-fallback coverage with hard-cutover
    rejection coverage.
- `crates/compass-history/src/lib.rs`
  - Export cache, reader, and graph-scan contracts.
- `crates/compass-history/src/store.rs`
  - Cache accessor; deletion of one-shot record reads; no change to
    authoritative publication validation.
- `crates/compass-history/src/diff.rs`
  - Move diff entry points to request-scoped readers and delete store-owned
    diff APIs.
- `crates/compass-core/src/pipeline.rs`
  - Accept an explicit shared cache root; use only the new cache constructor;
    remove legacy build-state validation.
- `crates/compass-core/src/history.rs`
  - Remove full validation from the existing-realization path; remove complete
    ancestor artifact seeding after shared cache activation; carry a prepared
    default viewer projection.
- `crates/compass-cli/src/history_build.rs`
  - Pass the history cache root into the child extractor.
- `crates/compass-cli/src/history_commands.rs`
  - Sealed status/profile/build reads, explicit `verify`, graph-only export,
    cache maintenance.
- `crates/compass-cli/src/semantic_diff_commands.rs`
  - Prefetched readers and report cache.
- `crates/compass-cli/src/semantic_commands.rs`
  - Move the semantic cache caller to the new constructor.
- `crates/compass-semantic/src/orchestration.rs`
  - Move semantic orchestration to the new constructor.
- `crates/compass-semantic-diff/src/lib.rs`
  - Export a comparison-engine cache version constant.
- `crates/compass-output/src/lib.rs`
  - Export historical viewer projection builder.
- `crates/compass-output/src/html.rs`
  - Share existing aggregation helpers with the streaming builder.
- `crates/compass-output/src/viewer_model.rs`
  - Add `Deserialize` only if a typed cached projection is required; otherwise
    cache canonical envelope bytes and leave the model serialize-only.

### Modified tests and qualification

- `crates/compass-files/src/cache.rs` unit tests.
- `crates/compass-files/tests/contracts.rs`.
- `crates/compass-semantic/src/tests.rs`.
- `crates/compass-semantic/tests/orchestration_coverage.rs`.
- `crates/compass-history/tests/publication.rs`.
- `crates/compass-history/tests/diff.rs`.
- `crates/compass-history/tests/maintenance.rs`.
- `crates/compass-history/tests/performance.rs`.
- `crates/compass-core/tests/history_materialize.rs`.
- `crates/compass-cli/tests/history_cli.rs`.
- `scripts/qualify_history_real_repo.sh`.
- `docs/guides/versioned-history.md`.
- `docs/reference/outputs.md`.
- `PERFORMANCE.md`.

---

### Task 1: Replace routine full validation with sealed fast reads

**Production files:**

- Modify: `crates/compass-core/src/history.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/src/history_build.rs`

**Coverage files:**

- Modify: `crates/compass-core/tests/history_materialize.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`
- Modify: `crates/compass-history/tests/publication.rs`

**Produces:**

- Constant-sized no-op build and profile lookup.
- An explicit integrity command:
  `compass history verify REV|REALIZATION [--format text|json]`.

- [ ] **Step 1: Implement the sealed existing-realization path**

In `observe_preferred`, accept `preferred_with_activity` after its existing
manifest parse, realization-ID recomputation, and direct-root verification.
Remove the unconditional `validate_with_activity` call.

In `resolve_or_materialize`, return an existing preferred realization after
`history.preferred(&commit)` without calling `history.validate`.

In `stored_profile`, copy `BuildProfile` from the sealed preferred/get result
without loading graph records.

In `execute_change_counts`, remove the two explicit `validate` calls.
`diff_records` already seal-checks both realizations and streams only requested
roots.

- [ ] **Step 2: Make ordinary status describe the seal honestly**

Change text status from `validation: valid` to `seal: valid`. Change JSON status
to:

```json
{
  "seal": {
    "valid": true,
    "mode": "manifest_and_direct_roots"
  }
}
```

Do not claim every record was revalidated. Emit only the new `seal` object; do
not retain or alias the old `validation` field. Incompatible stores, unreadable
manifests, missing roots, and preferred-pointer errors still fail closed.

- [ ] **Step 3: Add explicit full verification**

Dispatch `history verify` before the common export/status parser. Resolve its
single argument as a commit first and as a `RealizationId` second. For a commit,
verify the preferred realization. Call the existing `HistoryStore::validate`;
report all `ValidationReport` counts in JSON and a concise valid/invalid text
result.

Keep full validation in:

- `history verify`;
- `history prefer`;
- publication;
- corrupt-preferred recovery;
- explicit maintenance tests.

- [ ] **Step 4: Add regression coverage after production behavior exists**

Add tests proving:

- a valid published realization with a deliberately corrupted non-root record
  fails `history verify` but is not silently reconstructed by `history status`;
- a missing or mismatched direct root still fails status and no-op build;
- `history build REV --profile-from OTHER` reads the profile from the sealed
  manifest and does not call a graph builder;
- no-op materialization calls neither `CompleteGraphBuilder::build` nor
  `HistoryStore::artifacts`;
- `change-counts` remains byte-identical to the pre-change result.

Use a counting builder/test seam rather than wall-clock assertions for this
task.

- [ ] **Step 5: Verify the slice**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-history --test publication
cargo test -p compass-core --test history_materialize
cargo test -p compass-cli --test history_cli history_status
cargo test -p compass-cli --test history_cli profile_from
cargo test -p compass-cli --test history_cli change_counts
```

Expected: all pass; the no-op path does not enter full validation.

- [ ] **Step 6: Commit only this slice**

```bash
git add crates/compass-core/src/history.rs \
  crates/compass-cli/src/history_commands.rs \
  crates/compass-cli/src/history_build.rs \
  crates/compass-core/tests/history_materialize.rs \
  crates/compass-cli/tests/history_cli.rs \
  crates/compass-history/tests/publication.rs
git commit -m "perf: use sealed history fast paths"
```

---

### Task 2: Share portable AST and Program caches across historical worktrees

**Production files:**

- Modify: `crates/compass-files/src/cache.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Modify: `crates/compass-files/tests/contracts.rs`
- Add: `crates/compass-history/src/cache.rs`
- Modify: `crates/compass-history/src/lib.rs`
- Modify: `crates/compass-history/src/store.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/history.rs`
- Modify: `crates/compass-cli/src/history_build.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/src/semantic_commands.rs`
- Modify: `crates/compass-semantic/src/orchestration.rs`

**Coverage files:**

- Modify: `crates/compass-files/src/cache.rs` tests
- Modify: `crates/compass-files/tests/contracts.rs`
- Modify: `crates/compass-semantic/src/tests.rs`
- Modify: `crates/compass-semantic/tests/orchestration_coverage.rs`
- Modify: `crates/compass-core/tests/history_materialize.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Produces:**

- One owner-only content cache shared by all linked worktrees.
- Adjacent/arbitrary history builds that parse only cache misses.
- No complete ancestor artifact reconstruction.

- [ ] **Step 1: Implement the owner-only cache root and checked paths**

Create `<git-common-dir>/compass/cache/v1` through the same symlink rejection and
Unix owner-mode rules as the history store. `HistoryStore::cache()` must validate
every existing component before returning `HistoryCache`.

Implement derived envelope primitives now, even though semantic diff and viewer
will use them in later tasks. This keeps storage/security logic in one slice.

- [ ] **Step 2: Cut every cache caller over to `Cache::open`**

Implement `Cache::open(root, CacheOptions)` and migrate every `compass-files`
cache caller in:

- `compass-core/src/pipeline.rs`;
- `compass-semantic/src/orchestration.rs`;
- `compass-cli/src/semantic_commands.rs`;
- their unit and integration tests.

Delete `Cache::new`; do not leave a deprecated alias. Bump the deterministic
encoding namespace and delete:

- `legacy_directory`;
- the `allow_legacy` load parameter;
- JSON fallback for deterministic binary entries;
- Program JSON fallback;
- prompt-fingerprint fallback to an unversioned directory;
- pruning of legacy directories.

Remove `allow_legacy_validation` and the pre-build-state fast path from
`compass-core/src/pipeline.rs`. An output without a valid current build state is
rebuilt once under the new contract.

`CacheLayout::OutputDirectory` is the sole current-tree layout.
`CacheLayout::SharedHistory` writes `CacheKind` directories directly below
`cache/v1`.

- [ ] **Step 3: Implement verified shared-history hashing**

For shared AST lookup:

1. canonicalize the checkout root and candidate file;
2. require the file to remain below the checkout root;
3. normalize its relative path to `/`;
4. read file bytes;
5. hash relative path, bytes, extractor version, and encoding version;
6. use the digest as the entry filename.

Do not consult or flush `StatHashIndex` in this mode. Preserve existing
MessagePack decode limits, partial-entry rules, source-path rebasing, and atomic
writes.

Program cache keys remain caller-owned logical keys and include the
IR/provider/analyzer/merger versions already encoded by `CacheKind`.

- [ ] **Step 4: Pass the shared cache through the exact history builder**

Add `BuildOptions::cache_root: Option<PathBuf>`. Current-tree callers leave it
`None`. In `NativeCompleteGraphBuilder`, store the validated history
`extraction_root`, pass it to the child via a private
`COMPASS_HISTORY_CACHE_ROOT` environment variable, and set it on the child's
`BuildOptions` only when `COMPASS_HISTORY_BUILD=1`.

Reject a relative cache root. The parent, not the detached checkout, chooses the
path.

Both graph extraction and the parallel Program worker must construct a shared
history `Cache` using the same root. Temporary output remains under the worktree
and is deleted normally; cache entries survive.

Failure to create, validate, read, or write the shared extraction cache fails
the history build with the exact cache path and cause. Do not fall back to the
temporary output cache.

- [ ] **Step 5: Remove complete ancestor seeding**

Remove `compatible_seed` and the
`seed: Option<&GraphArtifacts>` parameter from `CompleteGraphBuilder::build`.
Delete `seed.write_seed(...)` from `NativeCompleteGraphBuilder`.

Do not replace it with another full `HistoryStore::artifacts` call. Historical
clustering already ignores prior communities under `COMPASS_HISTORY_BUILD`, so
the shared content cache contains the reusable state this build needs.

Delete first-parent seed/manifest selection from materialization. Profile and
fingerprint matching applies to the target realization only.

- [ ] **Step 6: Add hard-cutover, correctness, and reuse coverage**

After implementation, add tests proving:

- every production cache call site uses `Cache::open`;
- `Cache::new`, `legacy_directory`, `allow_legacy`, and
  `allow_legacy_validation` are absent from the workspace;
- legacy deterministic JSON and Program JSON files are ignored, not imported;
- a current output without the new build state performs one rebuild and then
  uses the new fast path;
- two different checkout roots with identical relative path and bytes map to
  the same shared entry;
- same size/mtime but different bytes cannot collide;
- the same bytes at different logical relative paths do not return stale
  `source_file` data;
- malformed MessagePack is a miss and is atomically replaced;
- parallel writers converge on one decodable entry;
- building adjacent commits extracts only the changed source file;
- cache-hit and cache-miss builds publish the same realization ID and canonical
  `graph.json`/`program.json`;
- the builder never calls `HistoryStore::artifacts` to obtain an ancestor seed.

- [ ] **Step 7: Verify the slice**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-files cache
cargo test -p compass-files --test contracts
cargo test -p compass-semantic
cargo test -p compass-core --test history_materialize
cargo test -p compass-cli --test history_cli history_build
cargo test -p compass-cli --test history_cli adjacent
```

Then run a local release smoke measurement in a temporary clone:

```bash
cargo build --release -p compass-cli
COMPASS_BIN="$PWD/target/release/compass" \
  scripts/qualify_history_real_repo.sh \
  /Volumes/workspace/Github/leveldb \
  78a352f47ed6c1e9d750545e9b242289185b87e1 \
  4a0c572440c7df2f56a6f5fb5aec9e366d522edb
```

Expected: the second build reports unchanged files as cache hits, and the
original checkout remains clean.

- [ ] **Step 8: Commit only this slice**

```bash
git add crates/compass-files/src/cache.rs \
  crates/compass-files/src/lib.rs \
  crates/compass-files/tests/contracts.rs \
  crates/compass-history/src/cache.rs \
  crates/compass-history/src/lib.rs \
  crates/compass-history/src/store.rs \
  crates/compass-core/src/pipeline.rs \
  crates/compass-core/src/history.rs \
  crates/compass-cli/src/history_build.rs \
  crates/compass-cli/src/history_commands.rs \
  crates/compass-cli/src/semantic_commands.rs \
  crates/compass-semantic/src/orchestration.rs \
  crates/compass-semantic/src/tests.rs \
  crates/compass-semantic/tests/orchestration_coverage.rs \
  crates/compass-core/tests/history_materialize.rs \
  crates/compass-cli/tests/history_cli.rs
git commit -m "perf: cut over to shared history cache"
```

---

### Task 3: Batch and memoize semantic evidence reads

**Production files:**

- Add: `crates/compass-history/src/reader.rs`
- Modify: `crates/compass-history/src/lib.rs`
- Modify: `crates/compass-history/src/store.rs`
- Modify: `crates/compass-history/src/diff.rs`
- Modify: `crates/compass-cli/src/semantic_diff_commands.rs`

**Coverage files:**

- Modify: `crates/compass-history/tests/diff.rs`
- Modify: `crates/compass-history/tests/performance.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Produces:**

- One seal check and one lazy root open per realization.
- At most one decode per evidence record per comparison.
- One reader-owned diff API with no one-shot/store-owned alternative.

- [ ] **Step 1: Implement `RealizationReader`**

Move typed lookup selection/decoding out of `HistoryStore::read_record` into the
request-scoped reader. Use interior mutability because `SnapshotReader` exposes
`&self` methods. Memoize missing records as well as present records.

Use a private owned key enum:

```rust
enum OwnedHistoryRecordKey {
    Node(String),
    ProgramModule(String),
    ProgramFunction(String),
    ProgramSummary(String),
    ReverseCallers(String),
}
```

Map node keys to the manifest's `nodes_root`; lazily load only
`program-facts`/`program-summaries` named roots when their first key is
requested.

After migrating callers, delete `HistoryStore::read_record`. Do not retain a
deprecated method or temporary-reader wrapper.

- [ ] **Step 2: Prefetch the first semantic evidence frontier**

In `semantic_diff_commands.rs`, replace `HistorySnapshots { store, old, new }`
with two `RealizationReader`s.

Before `compare`:

1. batch node IDs from direct node changes and dependency endpoints;
2. batch old/new module paths from `SourceFileDelta`;
3. collect symbol IDs from those modules and changed graph nodes;
4. batch functions, summaries, and reverse callers for those symbols;
5. collect returned caller IDs and batch those caller functions once.

The `SnapshotReader` implementation reads from the memoized readers and may
perform a bounded fallback lookup for evidence discovered later. It must not
reopen a manifest or root.

- [ ] **Step 3: Move root diff onto `RealizationReader`**

Implement `RealizationReader::diff` and
`RealizationReader::diff_records`. Node/edge diff and evidence lookups share the
same sealed `PublishedVersion` objects and activity window.

Migrate history change counts, semantic diff, and all history tests. Delete
`HistoryStore::diff` and `HistoryStore::diff_records`; do not leave forwarding
wrappers.

- [ ] **Step 4: Add operation-count and semantic parity tests**

After the production reader exists, add a test-only counter seam that records:

- manifest opens;
- named-root opens by kind;
- typed decodes by owned key;
- complete artifact reconstructions.

Assert one manifest open per side, at most one open for each requested root,
one decode per unique key, and zero artifact reconstructions.

Compare the new reader path with checked canonical fixture bytes; assert
identical `SemanticDiffReport` findings, stable IDs, source patches,
completeness, and limitations. Add a source scan asserting the removed
store-owned read/diff method names do not remain in production code.

- [ ] **Step 5: Verify the slice**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-history --test diff
cargo test -p compass-history --test performance
cargo test -p compass-cli --test history_cli semantic_diff
```

Expected: all pass; uncached diff performs no full realization reconstruction.

- [ ] **Step 6: Commit only this slice**

```bash
git add crates/compass-history/src/reader.rs \
  crates/compass-history/src/lib.rs \
  crates/compass-history/src/store.rs \
  crates/compass-history/src/diff.rs \
  crates/compass-history/tests/diff.rs \
  crates/compass-history/tests/performance.rs \
  crates/compass-cli/src/semantic_diff_commands.rs \
  crates/compass-cli/tests/history_cli.rs
git commit -m "perf: batch semantic history reads"
```

---

### Task 4: Cache canonical semantic-diff reports

**Production files:**

- Modify: `crates/compass-semantic-diff/src/lib.rs`
- Modify: `crates/compass-cli/src/semantic_diff_commands.rs`
- Modify: `crates/compass-history/src/cache.rs`

**Coverage files:**

- Modify: `crates/compass-cli/tests/history_cli.rs`
- Modify: `crates/compass-history/tests/maintenance.rs`

**Produces:**

- Repeated direction-sensitive semantic diff from one checked report payload.

- [ ] **Step 1: Define the comparison cache identity**

Export:

```rust
pub const SEMANTIC_DIFF_ENGINE_VERSION: u32 = 1;
```

Build canonical key material:

```json
{
  "schema": "compass.history.semantic_diff_key/1",
  "old_realization": "<id>",
  "new_realization": "<id>",
  "source_delta_sha256": "<digest>",
  "engine_version": 1,
  "report_schema": "compass.semantic_diff.report/1"
}
```

Hash `SourceFileDelta` using canonical JSON in Git order. Do not include
`--limit`, `--all`, `--explain`, output format, or output path; those are
rendering choices. Direction remains significant.

- [ ] **Step 2: Read the cache before graph diff/evidence work**

After resolving exact comparable realizations and source deltas:

1. build key material;
2. read and verify the cached payload;
3. deserialize `SemanticDiffReport`;
4. verify its old/new realization identities and schema;
5. render it with the current text/JSON/HTML options.

On a miss, execute Task 3's diff and evidence path, serialize the complete
report using canonical JSON, write it atomically, and then render the same
in-memory report.

Cache read/write failure should emit internal diagnostics when profiling is
enabled but must not invalidate authoritative history. A malformed hit is
ignored and replaced after recomputation.

- [ ] **Step 3: Add cache hit, invalidation, and determinism coverage**

After production behavior exists, assert:

- first and repeated JSON output are byte-identical;
- the repeated run performs zero Prolly graph diffs and zero evidence decodes;
- text `--limit`, text `--all`, `--explain`, JSON, and HTML reuse one report;
- swapping old/new produces a different key and correct reverse findings;
- changing either realization ID, source delta digest, engine version, or
  report schema misses;
- a truncated entry, digest mismatch, or report identity mismatch recomputes;
- parallel identical comparisons leave one valid payload.

- [ ] **Step 4: Verify the slice**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-history --test maintenance cache
cargo test -p compass-cli --test history_cli semantic_diff_cache
```

Measure one fixture twice with `scripts/measure_process.py`; expected second run
is dominated by report decode/render and remains byte-identical.

- [ ] **Step 5: Commit only this slice**

```bash
git add crates/compass-semantic-diff/src/lib.rs \
  crates/compass-history/src/cache.rs \
  crates/compass-history/tests/maintenance.rs \
  crates/compass-cli/src/semantic_diff_commands.rs \
  crates/compass-cli/tests/history_cli.rs
git commit -m "perf: cache semantic diff reports"
```

---

### Task 5: Stream and cache historical viewer projections

**Production files:**

- Add: `crates/compass-history/src/graph_read.rs`
- Modify: `crates/compass-history/src/lib.rs`
- Modify: `crates/compass-history/src/reader.rs`
- Add: `crates/compass-output/src/history_viewer.rs`
- Modify: `crates/compass-output/src/lib.rs`
- Modify: `crates/compass-output/src/html.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-core/src/history.rs`
- Modify: `crates/compass-cli/src/history_build.rs`

**Coverage files:**

- Modify: `crates/compass-history/tests/performance.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`
- Modify: `crates/compass-output/src/history_viewer.rs` tests

**Produces:**

- Graph view on cache miss without full authoritative reconstruction.
- Instant repeated overview/community exports.
- Default projection warming for newly published realizations.

- [ ] **Step 1: Implement graph-only record scanning**

Add `RealizationReader::scan_graph` with fixed root order:

1. analysis;
2. nodes;
3. edges.

Decode only node analysis fields, labels, `NodeRecord`, and `EdgeRecord`.
Validate key shapes, record limits, and typed schemas. An impossible analysis
reference or malformed authoritative record fails closed.

Do not open metadata merely to recover node/edge presentation order. Prolly
keys and the builder's final sort provide deterministic viewer ordering.

- [ ] **Step 2: Implement bounded projection construction**

Move the existing community aggregation logic in `html.rs` into reusable helpers
and implement `HistoricalViewBuilder`.

For an overview larger than `node_limit`, keep:

- one `HashMap<String, usize>` for node-to-community membership;
- one bounded community accumulator per community;
- one deterministic aggregate edge accumulator keyed by
  `(source_community, target_community, relation)`;
- labels and graph counts.

Do not retain every decoded `NodeRecord` or `EdgeRecord` after its accumulator
has been updated.

For an exact small graph or selected community, retain only nodes in scope and
edges whose endpoints are both retained. Apply the same deterministic sort and
dedup rules as the existing viewer.

- [ ] **Step 3: Cache canonical viewer envelopes**

Use key material:

```json
{
  "schema": "compass.history.viewer_key/1",
  "realization": "<id>",
  "viewer_schema": "compass.history.viewer_graph/1",
  "projection_version": 1,
  "node_limit": 5000,
  "community": null
}
```

On `history export REV --format json`:

1. seal-check the preferred realization;
2. read the cached complete envelope bytes;
3. on a hit, atomically copy those bytes to `--output`;
4. on a miss, stream graph roots, build the model, write canonical envelope
   bytes to cache, and atomically copy them to output.

Remove both the explicit `history.validate` and `history.artifacts` calls from
the viewer JSON path. Keep full artifact reconstruction for authoritative
`graph-json` and `compass-out` exports because those formats request the full
artifact contract.

- [ ] **Step 4: Warm the default overview during new publication**

Add a builder hook with a default `None` implementation that prepares default viewer bytes from the
already-loaded `CompletedGraphArtifacts` before publication moves those
artifacts:

```rust
fn default_viewer_projection(
    &self,
    completed: &CompletedGraphArtifacts,
    repository_root: &Path,
    commit: &CommitId,
) -> Result<Option<Vec<u8>>, MaterializeError> {
    Ok(None)
}
```

`NativeCompleteGraphBuilder` implements it with `node_limit=5000` and no
community. After the authoritative publication succeeds and the realization ID
is known, write the projection under the final realization-derived key.

Projection generation or cache publication failure must not roll back or mark
an otherwise valid realization corrupt. Every missing projection uses the same
streaming miss path; there is no realization-age check, lazy-upgrade branch, or
migration marker.

- [ ] **Step 5: Add projection parity and root-bound coverage**

After implementation, assert:

- cached and uncached viewer envelope bytes are identical;
- the streaming model equals `graph_view_model_document` for small, aggregated,
  and selected-community fixtures;
- a cache miss opens analysis/nodes/edges once and never opens hyperedges,
  metadata, Program facts, or Program summaries;
- a cache hit opens no record roots;
- deleting any projection causes deterministic regeneration without changing
  the realization ID;
- corrupt projection bytes regenerate;
- corrupt authoritative graph records fail closed;
- default projection exists after a new successful build;
- a 120k-node synthetic graph remains below the operation-count/memory bound in
  the ignored release performance test.

- [ ] **Step 6: Verify the slice**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-output history_viewer
cargo test -p compass-history --test performance viewer
cargo test -p compass-cli --test history_cli history_export
npm test -w compass-vscode
npm run typecheck -w compass-vscode
```

Expected: the CLI envelope schema and extension command contract use only the
new cutover behavior; no dual response or deprecated command path remains.

- [ ] **Step 7: Commit only this slice**

```bash
git add crates/compass-history/src/graph_read.rs \
  crates/compass-history/src/lib.rs \
  crates/compass-history/src/reader.rs \
  crates/compass-history/tests/performance.rs \
  crates/compass-output/src/history_viewer.rs \
  crates/compass-output/src/lib.rs \
  crates/compass-output/src/html.rs \
  crates/compass-core/src/history.rs \
  crates/compass-cli/src/history_build.rs \
  crates/compass-cli/src/history_commands.rs \
  crates/compass-cli/tests/history_cli.rs
git commit -m "perf: stream historical viewer projections"
```

---

### Task 6: Add explicit cache status and garbage collection

**Production files:**

- Modify: `crates/compass-history/src/cache.rs`
- Modify: `crates/compass-history/src/lib.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`

**Coverage files:**

- Modify: `crates/compass-history/tests/maintenance.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Produces:**

- Non-destructive cache inspection.
- Explicit, dry-run-by-default budget/age pruning.

- [ ] **Step 1: Implement deterministic cache inventory**

Walk only known `cache/v1` namespaces without following symlinks. Report file
count and allocated payload bytes per namespace. Ignore temporary atomic-write
files younger than an active operation; report malformed/unsafe paths as
diagnostics rather than traversing them.

Touch an entry's access timestamp only after a verified cache hit. Do not touch
on a malformed miss.

- [ ] **Step 2: Implement GC planning and sweep**

Support:

```text
compass history cache status [--format text|json]
compass history cache gc
  [--max-bytes N]
  [--max-age-days N]
  [--format text|json]
  [--yes]
```

Rules:

- `cache gc` is a dry run without `--yes`;
- at least one of `--max-bytes` or `--max-age-days` is required;
- remove obsolete schema/extractor namespaces first;
- remove expired derived entries next;
- enforce the remaining byte budget by least-recent verified access;
- never remove `history.sqlite` or named Prolly roots;
- acquire `HistoryStore::maintenance()` for the sweep;
- validate every planned path is a regular file below the exact cache root
  before deletion.

Do not add automatic deletion to ordinary `history gc`.

- [ ] **Step 3: Add maintenance safety coverage**

After production code exists, test:

- status totals by namespace;
- dry run changes nothing;
- `--yes` removes exactly the plan;
- age and byte policies compose deterministically;
- an activity guard blocks cache GC;
- symlink and path-escape attempts fail closed;
- active/current-schema entries survive when below budget;
- immutable realizations remain readable after deleting the entire cache.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-history --test maintenance cache
cargo test -p compass-cli --test history_cli history_cache
```

Then:

```bash
git add crates/compass-history/src/cache.rs \
  crates/compass-history/src/lib.rs \
  crates/compass-history/tests/maintenance.rs \
  crates/compass-cli/src/history_commands.rs \
  crates/compass-cli/tests/history_cli.rs
git commit -m "feat: add history cache maintenance"
```

---

### Task 7: Document the fast paths and make qualification reproducible

**Files:**

- Modify: `scripts/qualify_history_real_repo.sh`
- Modify: `docs/guides/versioned-history.md`
- Modify: `docs/reference/outputs.md`
- Modify: `PERFORMANCE.md`
- Modify: `crates/compass-history/tests/performance.rs`

**Produces:**

- One real-repository command that measures cold/warm build, first/repeated
  semantic diff, and first/repeated viewer export.

- [ ] **Step 1: Extend the existing qualification script**

Keep the existing clean-checkout and shared-clone protections. Use
`scripts/measure_process.py` for every timed child and emit a canonical JSON
summary containing:

```json
{
  "repository": "...",
  "old": "...",
  "new": "...",
  "binary": "...",
  "operations": {
    "current_cold": {"seconds": 0.0, "peak_rss_kib": 0},
    "current_incremental": {"seconds": 0.0, "peak_rss_kib": 0},
    "history_cold": {"seconds": 0.0, "peak_rss_kib": 0},
    "history_adjacent": {"seconds": 0.0, "peak_rss_kib": 0},
    "history_noop": {"seconds": 0.0, "peak_rss_kib": 0},
    "semantic_first": {"seconds": 0.0, "peak_rss_kib": 0},
    "semantic_repeat": {"seconds": 0.0, "peak_rss_kib": 0},
    "viewer_first": {"seconds": 0.0, "peak_rss_kib": 0},
    "viewer_repeat": {"seconds": 0.0, "peak_rss_kib": 0}
  },
  "digests": {
    "semantic_first": "...",
    "semantic_repeat": "...",
    "viewer_first": "...",
    "viewer_repeat": "..."
  },
  "original_checkout_clean": true
}
```

Resolve the release binary to an absolute path. Use a temporary shared clone,
never reset or clean the supplied source repository, and verify its porcelain
status is unchanged at the end.

- [ ] **Step 2: Add ignored release-scale gates**

Extend `crates/compass-history/tests/performance.rs` with ignored tests for:

- 120k nodes / 260k edges graph-only projection;
- root-equal Prolly diff;
- sparse node/edge diff;
- 100k memoized evidence lookups;
- derived cache hit decode.

Operation-count assertions run normally; strict elapsed/RSS assertions remain
ignored and are invoked during release qualification.

- [ ] **Step 3: Update user and maintainer documentation**

Document:

- first-ever arbitrary revision performs bounded extraction;
- the release is a hard cutover with an empty new cache namespace;
- old cache files and old build state are ignored rather than migrated;
- there is no compatibility flag, cache importer, or dual CLI response;
- authoritative realization schema remains current because its format did not
  change;
- repeated build/diff/view behavior;
- `history verify` versus sealed `history status`;
- `history cache status/gc`;
- cache location and disposability;
- viewer projection and community lazy-load behavior;
- performance commands and target table.

Do not document internal environment variables as public API.

- [ ] **Step 4: Verify and commit**

Run:

```bash
shellcheck scripts/qualify_history_real_repo.sh
cargo test -p compass-history --test performance
git diff --check
```

Then:

```bash
git add scripts/qualify_history_real_repo.sh \
  docs/guides/versioned-history.md \
  docs/reference/outputs.md \
  PERFORMANCE.md \
  crates/compass-history/tests/performance.rs
git commit -m "docs: add versioned history performance gates"
```

---

### Task 8: Run the full correctness and real-repository acceptance gate

**Files:**

- Add:
  `docs/superpowers/reviews/2026-07-26-versioned-history-performance-qualification.md`
- Refresh: `graphify-out/`

**Produces:**

- Recorded CocoIndex and Podman results.
- Pushed implementation commits and updated pull request.

- [ ] **Step 1: Run the complete Rust gate**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
```

If repository-wide tests contain known unrelated failures, rerun every affected
package/test named in Tasks 1–7 and record the unrelated failure verbatim. Do
not describe the full gate as passing.

- [ ] **Step 2: Run extension and viewer gates**

Run:

```bash
npm test -w compass-vscode
npm run typecheck -w compass-vscode
npm test -w @compass/viewer-tests
```

Confirm extension source still launches only Compass history/diff CLI commands.

- [ ] **Step 3: Build the release binary**

Run:

```bash
cargo build --release -p compass-cli
```

Record:

```bash
target/release/compass --version
git rev-parse HEAD
uname -a
```

- [ ] **Step 4: Qualify CocoIndex**

Use the already-established clean CocoIndex source repository and the approved
revision pair:

```bash
COMPASS_BIN="$PWD/target/release/compass" \
  scripts/qualify_history_real_repo.sh \
  /Volumes/workspace/Github/cocoindex-compass-audit-20260726 \
  90571539fa291fc6e6b248095bd2c8a2ff68bab4 \
  71f9cc9dc693080310181a2d011fb737420f7907 \
  > target/cocoindex-versioned-history-performance.json
```

Run three warm samples for no-op build, repeated semantic diff, and repeated
viewer export; report the median. Verify first/repeat semantic and viewer
digests match.

- [ ] **Step 5: Qualify Podman**

Resolve the clean local Podman checkout and choose two adjacent commits that
both contain the supported source corpus. Run:

```bash
COMPASS_BIN="$PWD/target/release/compass" \
  scripts/qualify_history_real_repo.sh \
  /Volumes/workspace/Github/podman \
  d8380c9c80d9c4acf5afd59b65c4c779aaacbbf5 \
  7ac3e837075460ecdea5ce59e607cdaa6b6709fc \
  > target/podman-versioned-history-performance.json
```

Record the resolved commits in the qualification document. Run three warm
samples and report medians. Verify semantic diff/view peak RSS is below 512 MiB
and build RSS is within 25% of current extraction.

- [ ] **Step 6: Write the qualification report**

Create the review document with:

- machine, OS, binary version, Compass commit;
- exact repository paths and revision IDs;
- before/after table for every operation;
- cache hit/miss counts;
- root opens, validation calls, and reconstruction calls;
- output SHA-256 digests;
- checkout cleanliness evidence;
- each accepted SLO and any explicit gap with cause/follow-up.

Do not claim an SLO passed without a captured measurement.

- [ ] **Step 7: Refresh the required knowledge graph**

Run from `/Users/haipingfu/graphify/compass`:

```bash
graphify update .
```

Then verify:

```bash
git status --short
git diff --check
```

Do not stage unrelated pre-existing files or generated directories unless they
are already tracked and changed solely by this implementation.

- [ ] **Step 8: Commit qualification evidence**

```bash
git add docs/superpowers/reviews/2026-07-26-versioned-history-performance-qualification.md
git commit -m "docs: qualify versioned history performance"
```

- [ ] **Step 9: Push and update the existing pull request**

Inspect the exact branch and PR before mutation:

```bash
git status --short --branch
git log --oneline --decorate -12
gh pr view 50 --json number,url,headRefName,baseRefName,state
```

Push the current branch and add a concise qualification comment:

```bash
git push origin HEAD
gh pr comment 50 \
  --body-file docs/superpowers/reviews/2026-07-26-versioned-history-performance-qualification.md
```

If PR #50 is closed or no longer targets this branch, stop and report the exact
state instead of silently creating a different PR.

---

## Final acceptance checklist

- [ ] First unseen arbitrary commits complete through the protected bounded
  extraction path.
- [ ] Adjacent commits reuse portable AST and Program entries across temporary
  worktree roots.
- [ ] No-op build and `--profile-from` perform sealed manifest/direct-root reads
  only.
- [ ] Explicit `history verify` retains complete validation.
- [ ] Ordinary diff reconstructs neither complete graph.
- [ ] Each semantic evidence record is decoded at most once per first
  comparison.
- [ ] Repeated semantic diff uses one checked canonical report.
- [ ] Viewer miss opens only analysis/nodes/edges; viewer hit opens no record
  roots.
- [ ] Cached/uncached realization, semantic report, and viewer digests match.
- [ ] Derived cache corruption regenerates; authoritative corruption fails
  closed.
- [ ] Cache GC is explicit, dry-run by default, and cannot remove immutable
  history.
- [ ] No legacy cache decoder, legacy directory lookup, `Cache::new`,
  `allow_legacy`, `allow_legacy_validation`, `HistoryStore::read_record`,
  `HistoryStore::diff`, or `HistoryStore::diff_records` remains.
- [ ] Status emits only the new seal contract; no deprecated validation field
  or compatibility alias remains.
- [ ] The release has no feature flag, dual read/write, cache import, or
  migration path.
- [ ] CocoIndex and Podman source checkouts remain unchanged.
- [ ] Release latency/RSS targets are recorded honestly.
- [ ] Rust, VS Code, and viewer regression gates pass or unrelated failures are
  documented exactly.
- [ ] `graphify update .` has refreshed the required development graph.
- [ ] Only intended files are committed, pushed, and reflected in the existing
  pull request.
