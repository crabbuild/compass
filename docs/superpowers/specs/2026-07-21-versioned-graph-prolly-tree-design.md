# How Compass stores versioned graphs with SQLite-backed Prolly trees

**Date:** 2026-07-21

**Revised:** 2026-07-22 after the Compass workspace extraction and command flattening

**Status:** Approved architecture, revised after storage, correctness, and Compass-rename audits

**Implementation root:** `/Users/haipingfu/graphify/compass` (the `crabbuild/compass` Git submodule and standalone repository)

**Pinned dependencies:** `prolly-map = "=0.5.0"` and `prolly-store-sqlite = "=0.3.0"`

## Purpose and scope

This design defines a local versioned graph map for Compass, the standalone native Rust code-intelligence system extracted from Graphify. It stores one complete immutable graph realization for each materialized Git commit, supports historical query, and streams graph-aware differences between realizations. `compass` is the sole canonical command surface. A legacy `graphify` executable may remain as a best-effort transition shim, but its command availability, help, output, exit status, and side effects do not constrain Compass or this feature's acceptance criteria.

A complete realization includes Abstract Syntax Tree (AST) facts, semantic nodes and edges, inferred relationships, hyperedges, community assignments, analysis, and the metadata required to reconstruct Compass's Graphify-compatible outputs. Git remains the source of source-code history. SQLite-backed Prolly trees store graph history without placing generated data in Git commits.

## Approved decisions

- Every preferred realization represents a complete Compass build, including semantic and inferred relationships
- History is explicitly enabled and disabled with `compass history enable|disable`; merely inspecting status or running an explicit historical command never silently enables eager work
- Once enabled, new commits enqueue eager materialization after `git commit` without blocking the commit on extraction
- Build, query, path, explain, export, and diff synchronously materialize missing commits from their exact Git trees; inspection-only status/list/show never trigger extraction
- One Git commit may have multiple immutable realizations
- Each realization records an extraction fingerprint for every meaning-affecting input
- One realization per commit is preferred without deleting other realizations
- One realization contains separate Prolly trees for nodes, edges, hyperedges, analysis, and metadata
- `prolly-store-sqlite` is the only v1 Prolly backend
- Implementation lives in the standalone Compass workspace (`compass/` in the Graphify superproject); crates and Rust modules use `compass-*` and `compass_*` names
- All linked worktrees share one SQLite database below the Git common directory
- New history-owned filesystem and Prolly namespaces use `compass`, including `<git-common-dir>/compass/`, `compass/store-format/v1`, and `compass/v1/...`
- Operational jobs, leases, locks, and temporary worktrees remain files beside the database
- `graph.json` remains a compatibility export
- Historical compatibility means canonical semantic equivalence, not byte-for-byte output reproduction
- A versioned artifact registry classifies authoritative, derived, and operational outputs
- Non-derivable authoritative sidecars are stored verbatim; reports and HTML are regenerated
- Historical ignores come only from the target commit and explicit build-profile excludes
- Temporary historical checkouts disable hooks, network fetching, and LFS smudging
- One cross-process reader-writer lock coordinates activity and maintenance
- Raw token usage, costs, timings, and diagnostics remain operational attempt provenance outside realization identity
- Replacing a corrupt preferred realization requires explicit `--replace-corrupt` recovery
- History remains opt-in until semantic-correctness, durability, and performance gates pass
- Compass command contracts are specified and tested independently; no Graphify CLI compatibility or alias-parity gate applies
- Every canonical user command begins directly with `compass`; there is no legacy binary or nested graph-command namespace
- Established Graphify-compatibility resources remain unchanged: `graphify-out/`, `graph.json`, `.graphifyignore`, `.graphify_*` sidecars, and `GRAPHIFY_*` environment variables

## Naming and resource boundary

The Compass extraction changes implementation ownership and the canonical command surface. It does not rename established Graphify-compatible artifact and configuration contracts.

| Concern | V1 canonical name | Treatment |
|---|---|---|
| Standalone repository and superproject path | `crabbuild/compass`, `compass/` | All implementation work and commits occur in the Compass repository |
| Workspace packages and Rust modules | `compass-*`, `compass_*` | New history code follows Compass naming |
| Executable and command prefix | `compass` | Commands are flat, such as `compass query` and `compass history build`; never `compass graph ...` |
| History-owned local directory and database | `<git-common-dir>/compass/history.sqlite` | New Compass resource; shared by linked worktrees |
| History-owned Prolly names | `compass/store-format/v1`, `compass/v1/...` | New Compass namespace |
| Legacy executable | `graphify` | Optional best-effort transition shim; it is outside the versioned-graph public contract and may diverge or be removed |
| Compatibility artifacts and configuration | `graphify-out/`, `graph.json`, `.graphifyignore`, `.graphify_*`, `GRAPHIFY_*` | Retained verbatim so existing consumers and repositories continue to work |

## Goals

1. Query, traverse, and explain the complete graph at any materialized commit.
2. Materialize a missing historical commit from its exact Git tree.
3. Generate new commit graphs without blocking `git commit` on extraction.
4. Diff graph topology and attributes through shared Prolly subtrees.
5. Preserve provenance and distinguish different extraction environments.
6. Reconstruct Compass's graph and sidecar outputs without semantic or structural loss.
7. Prevent incomplete, corrupt, or unsupported graphs from becoming preferred.
8. Recover deterministically from process termination, write contention, and stale jobs.
9. Keep realization identity independent from operational cost and timing variation.
10. Regenerate supported presentation artifacts from versioned semantic state.

## Non-goals and explicit limits

- This design does not replace Git branches, commits, merges, or synchronization.
- This design does not store the SQLite database in ordinary Git commits.
- V1 does not synchronize graph history between machines.
- V1 does not offer a runtime storage-backend selector.
- V1 does not make semantic extraction deterministic.
- V1 does not introduce per-file Prolly roots.
- V1 does not automatically delete realizations after Git history rewrites.
- V1 rejects mutable external sources such as live Google Workspace and PostgreSQL inputs. A later design must snapshot those inputs before they can represent a Git commit.
- V1 materializes committed superproject files. It does not fetch Git submodules or Git Large File Storage (LFS) objects. Status output reports detected gitlinks or LFS pointers.
- V1 applies committed `.gitignore` and `.graphifyignore` files plus explicit normalized build-profile excludes. It does not apply `.git/info/exclude` or global Git ignore configuration.
- SQLite garbage collection makes deleted pages reusable. It does not promise that the database file immediately shrinks.

## Existing Compass boundaries

Compass already separates the relevant responsibilities:

- `compass-model` owns `GraphDocument`, `NodeRecord`, and `EdgeRecord`
- `compass-core` produces the resolved, inferred, clustered, and validated graph
- `compass-output` writes NetworkX-compatible `graph.json` and sidecar artifacts
- `compass-query` consumes an immutable query-oriented `Graph`
- `compass-files` owns manifests, hashes, atomic file writes, and incremental detection
- `compass-graph` defines current graph-aware identity and analysis behavior

The history layer consumes completed artifacts. It does not parse code, prompt a model, resolve symbols, deduplicate entities, cluster nodes, or infer relationships.

## Component architecture

Add a `compass-history` workspace crate. It depends on `compass-model`, `compass-files`, `prolly-map`, and `prolly-store-sqlite`. It does not depend on CLI presentation or semantic-provider crates.

`compass-history` owns:

- canonical graph-record encoding
- typed Prolly key construction
- extraction-fingerprint calculation
- immutable realization manifests
- SQLite-backed Prolly roots and catalogs
- lossless artifact decomposition and reconstruction
- streaming graph-aware diffs
- realization validation and garbage collection
- repository discovery, revision resolution, jobs, leases, and maintenance locks

`compass-core` owns reusable materialization orchestration behind an injected complete-graph builder. `compass-cli` supplies the production builder, handles commands, starts workers, and renders progress and diagnostics.
`compass-output` owns the version-dispatched derived-artifact renderers. Its history bundle API
accepts model/JSON inputs rather than `compass-history` types, avoiding a dependency cycle;
`compass-cli` joins reconstructed artifacts and registry entries to that API.

## SQLite storage boundary

### Local layout

Resolve the shared directory with `git rev-parse --git-common-dir`. Never assume `.git` is a directory because linked worktrees use a `.git` file.

```text
<git-common-dir>/compass/
├── history.sqlite
├── history.sqlite-wal
├── history.sqlite-shm
├── config.json
├── jobs/
├── leases/
├── locks/
└── tmp/
```

SQLite creates the `-wal` and `-shm` files while Write-Ahead Logging (WAL) is active. Callers must not treat the main file alone as a consistent live backup. Compass exports realizations through its history commands instead of recommending raw database copies.

On Unix, Compass creates the `compass` directory with mode `0700` and the database and operational files with mode `0600`. Other platforms use their closest owner-only controls. Opening rejects a symlink at the `compass` directory, database, lock, configuration, job, lease, or temporary-worktree target; cleanup only removes validated descendants of the canonical `tmp` directory. The store never contains credentials, but it can contain proprietary source-derived graph data.

### Adapter configuration

The concrete store is `prolly_store_sqlite::SqliteStore`. `HistoryStore` constructs it with `SqliteStore::open_with_config` and wraps it in `prolly::Prolly`:

```rust
pub struct HistoryStore {
    root: PathBuf,
    engine: Prolly<Arc<SqliteStore>>,
}
```

V1 uses these settings:

- `enable_wal: true`
- `synchronous_normal: false`, which retains SQLite's full synchronous default
- a 10-second busy timeout

The adapter owns the SQLite schema. Compass does not read or write `prolly_nodes`, `prolly_hints`, or `prolly_roots` through direct SQL. This prevents Compass from coupling to private adapter details.

`SqliteStore` implements `Store`, `NodeStoreScan`, `ManifestStore`, `ManifestStoreScan`, and `TransactionalStore`. These contracts support content-addressed nodes, root listing, compare-and-swap, strict multi-root transactions, and Prolly garbage collection.

The adapter audit used the published `prolly-store-sqlite 0.3.0` source, which depends on `prolly-map 0.5.0`. Its 18 unit and integration tests plus two documentation tests passed locally, including strict transaction rollback and file-backed reopen coverage. The [adapter manifest](https://github.com/crabbuild/prolly/blob/main/stores/prolly-store-sqlite/Cargo.toml) and [SQLite adapter documentation](https://github.com/crabbuild/prolly/blob/main/stores/prolly-store-sqlite/README.md) remain the primary upstream references.

### Project-owned API

Business logic depends on `HistoryStore`, not on `SqliteStore` or raw Prolly types. The public boundary uses project types:

```rust
impl HistoryStore {
    pub fn create(repository: &Repository) -> Result<Self, HistoryError>;
    pub fn open_existing(repository: &Repository)
        -> Result<Option<Self>, HistoryError>;
    pub fn publish(&self, request: PublishRequest)
        -> Result<PublishedVersion, HistoryError>;
    pub fn preferred(&self, commit: &CommitId)
        -> Result<Option<PublishedVersion>, HistoryError>;
    pub fn get(&self, id: &RealizationId)
        -> Result<PublishedVersion, HistoryError>;
    pub fn diff(&self, old: &RealizationId, new: &RealizationId,
        sink: &mut dyn ChangeSink) -> Result<(), HistoryError>;
}
```

`create` is used only by enabling history or by an explicit build/materialization operation. `open_existing` never creates the history directory or database, so read-only status and list operations cannot enable history or mutate an untouched repository. Store existence and eager-history enablement are independent: disabling eager history retains all realizations and still permits explicit build, query, diff, export, validation, and garbage collection.

The wrapper persists each tree root as its Content Identifier (CID) plus its persisted `TreeFormat`. It never includes runtime cache settings in stored identity.

### Store format compatibility

The database contains a reserved named root with this logical name:

```text
compass/store-format/v1
```

Its canonical value records the Compass history schema, canonical encoding version, typed-key version, and expected adapter family. Opening an existing database verifies this record before reading application roots. Unknown or incompatible versions fail with a migration-required error.

Dependency versions are exact pins. Upgrading either Prolly crate requires contract tests, reopen tests against an old fixture database, and an explicit migration decision.

### Contention and failures

Each process opens its own `SqliteStore`; the adapter serializes calls within that process. WAL and the busy timeout coordinate multiple processes. A store error never triggers an unbounded blind retry.

Interactive commands report a bounded diagnostic and preserve the prior preferred realization. Background jobs remain retryable with an incremented attempt count. Strict transaction conflicts follow the catalog rules below rather than being treated as database corruption.

## Version and identity model

### Completion evidence

A file's existence does not prove a complete build. `.graphify_semantic_marker` records token use and cannot serve as a success marker.

Every publication supplies content-addressed completion evidence:

```rust
struct CompletionEvidence {
    extraction_succeeded: bool,
    allow_partial: bool,
    semantic_files_expected: u64,
    semantic_files_completed: u64,
    failed_chunks: u64,
}
```

Completion evidence is authoritative realization content. Raw token usage, provider cost,
timings, captured diagnostics, and the original `.graphify_semantic_marker` are operational
attempt provenance and do not enter any identity-bearing tree. A compatibility export may
generate a semantic marker from normalized completion evidence. Operational attempt records
remain subject to job-retention policy and are not permanent graph-history artifacts.

Publication requires a successful extraction, `allow_partial == false`, equal expected and completed semantic-file counts, and zero failed chunks. A repository with no semantic-source files has equal zero counts and remains complete.

Publication cross-checks these counts against the extraction manifest and semantic-source inventory produced from the exact worktree. `PublishRequest` is an internal materialization boundary, not a command that accepts caller-supplied completion claims.

History builds reject `--allow-partial`, `--code-only`, and `--no-cluster`. They also reject unsnapshotted external-source flags. The normal pipeline still supports those flags outside history mode.

### Graph realization manifest

A published realization has this immutable manifest:

```rust
struct GraphVersion {
    schema_version: u32,
    git_commit: String,
    git_parents: Vec<String>,
    extraction_fingerprint: String,
    nodes_root: TreeRoot,
    edges_root: TreeRoot,
    hyperedges_root: TreeRoot,
    analysis_root: TreeRoot,
    metadata_root: TreeRoot,
    node_count: u64,
    edge_count: u64,
    hyperedge_count: u64,
    analysis_count: u64,
    metadata_count: u64,
}
```

`RealizationId` is the SHA-256 digest of canonical manifest bytes. Operational values such as time, machine path, process ID, job ID, and duration do not enter the manifest.

An identical rebuild returns the same realization ID only when the commit, parents, fingerprint, persisted tree formats, and graph content are identical. Identical graph content at different Git commits produces different IDs because commit provenance is part of the manifest.

The manifest records Git parent commit IDs, not parent realization IDs. Compass can therefore materialize a child before its parent without rewriting the child's immutable manifest.

### Extraction fingerprint

The extraction fingerprint hashes normalized inputs that can change graph meaning:

- Compass, graph-schema, extractor, resolver, semantic-pipeline, and analysis versions
- semantic prompt digest
- provider and model identifiers
- extraction mode and meaning-affecting flags
- directed or undirected mode
- multigraph behavior
- clustering algorithm and configuration
- repository-local Compass/Graphify-compatible configuration and ignore-file contents from the target commit
- enabled optional extractors and compile-time features

The fingerprint excludes credentials, credential environment-variable values, absolute paths, timestamps, machine names, concurrency, timeouts, and logging options. The job record may contain non-secret runtime limits.

The commit identifies source content while the fingerprint identifies the extraction environment. Two runs with the same commit and fingerprint may still produce different semantic content. Both realization IDs remain addressable.

## Catalog and publication model

### Named roots

All logical catalog entries are binary, segment-safe named roots inside SQLite's `prolly_roots` table. Human-readable forms below describe their meaning:

```text
compass/v1/version/<realization-id>/nodes
compass/v1/version/<realization-id>/edges
compass/v1/version/<realization-id>/hyperedges
compass/v1/version/<realization-id>/analysis
compass/v1/version/<realization-id>/metadata
compass/v1/version/<realization-id>/manifest
compass/v1/preferred/<commit>
```

The preferred root points to the selected realization's manifest tree. The manifest records the commit and fingerprint, so listing code validates catalog names against content instead of trusting root names.

### Publication sequence

Only complete and validated content enters the version catalog:

1. Build the five typed trees and manifest tree.
2. Validate direct immutable roots, counts, endpoints, schema, and completion evidence.
3. Publish the six realization roots in one strict SQLite transaction.
4. Reopen the realization through its catalog roots and verify its digest and roots.
5. Compare-and-swap `preferred/<commit>` from the value observed before publication.
6. Record the job as published, including whether this realization became preferred.

Prolly builders may write content-addressed nodes before step 3. Those nodes remain unreachable if validation or publication fails and normal garbage collection can remove them.

The realization-root transaction and preferred compare-and-swap are separate by design. If two different realizations publish concurrently, both remain addressable. A stale preferred compare-and-swap does not overwrite the winner and returns `preferred: false` for the losing publication.

Before attempting step 5, publication validates the observed preferred realization when one exists. A corrupt preferred root prevents automatic replacement, leaves the new realization addressable but non-preferred, and returns a corruption diagnostic. Only `compass history rebuild <revision> --replace-corrupt` may replace a corrupt preferred root; ordinary `compass history prefer` fails closed on corrupt current state.

An explicit `compass history prefer` command reads the current preferred ID and performs the same compare-and-swap. A concurrent change produces a conflict instead of an implicit overwrite.

Readers resolve only named realization roots or preferred roots. They never infer published state from unreferenced content-addressed nodes.

## Typed graph maps

Every key starts with a schema-version byte and record-kind byte. Remaining components use Prolly's length-prefixed `KeyBuilder` segments. Arbitrary Unicode, NUL bytes, separators, and common prefixes cannot collide.

### Node map

```text
key:   schema / node / canonical node ID
value: complete canonical NodeRecord excluding separately stored analysis fields
```

Node IDs remain case-sensitive strings. Unknown attributes remain in the value. Duplicate node IDs fail validation.

### Edge map

For `multigraph: false`, the key is:

```text
schema / edge / direction-aware source / target / relation
```

Directed keys retain endpoint order. Undirected keys sort endpoints for identity while the value retains the producer's source and target. Duplicate non-multigraph identities fail validation.

For `multigraph: true`, append a discriminator:

```text
schema / edge / direction-aware source / target / relation / discriminator
```

Use a canonical NetworkX `key` attribute when present. Otherwise, use the SHA-256 digest of the complete canonical edge followed by its zero-based occurrence among byte-identical records. This rule preserves parallel edges and exact duplicates. An attribute change on a digest-keyed parallel edge appears as removal plus addition.

### Hyperedge map

```text
key:   schema / hyperedge / explicit ID or canonical-record digest
value: complete canonical hyperedge record
```

Use a valid explicit `id` when present. Duplicate explicit IDs fail validation. Otherwise,
append the zero-based occurrence among byte-identical id-less records to the complete
canonical-record digest:

```text
schema / hyperedge / canonical-record-digest / occurrence
```

This preserves exact duplicate array entries. Preserve member-array order and unknown
attributes in the value. Validation recognizes Compass's supported `nodes` and `members`
forms and verifies every referenced node.

### Analysis map

The analysis tree stores every graph-derived field and analysis sidecar record needed for reconstruction and architectural diff:

```text
community/<node-id>
community-label/<community-id>
cohesion/<community-id>
god-node/<rank>
surprise/<rank>
question/<rank>
sidecar/<typed-path>
```

Known records receive stable logical keys. Unknown analysis fields receive versioned typed paths and retain their complete canonical JSON values. No analysis needed for compatibility remains only in an unversioned worktree file.

### Metadata map

The metadata tree stores:

- `directed` and `multigraph`
- unknown `graph` members and unknown top-level fields
- whether the document used `links` or legacy `edges`
- node, edge, and hyperedge input-order records
- original hyperedge placement
- labels, extraction manifest, normalized semantic provenance, and completion evidence
- graph and export schema versions
- values required to reproduce Compass's supported outputs

Order records use lexicographically ordered numeric ranks and reference typed record keys, including multigraph discriminators. This keeps values bounded while preserving exact array ordering. Unknown arrays retain their original order.

### Versioned artifact registry

The metadata tree contains one registry record for every supported realization artifact:

```text
artifact/<relative-path> -> {
  registry_version,
  class,
  media_type,
  schema_version,
  content_digest,
  storage,
  regeneration_version
}
```

`class` is one of `authoritative`, `derived`, or `operational`:

- Authoritative graph state includes the decomposed `graph.json`, analysis, labels,
  extraction manifest, completion evidence, and any non-derivable semantic sidecar. Its
  canonical content or verbatim bytes are stored in the five realization trees and affect
  the realization ID.
- Derived artifacts include `GRAPH_REPORT.md`, HTML, visualization output, label signatures,
  and other deterministic presentations. The registry records their regeneration version;
  their generated bytes do not affect realization identity.
- Operational artifacts include `.graphify_root`, learning overlays, query logs, caches,
  incomplete markers, chunk temporaries, raw semantic markers, `cost.json`, timings, and
  diagnostics. They are excluded from realization content. Attempt provenance may retain a
  bounded subset outside the Prolly catalog.

Historical export guarantees canonical semantic equivalence. It does not guarantee that a
regenerated report or HTML file is byte-identical to the original run. Unknown future
sidecars must be classified explicitly; the loader never silently drops an unclassified
file that the current artifact contract declares authoritative.

## Canonical encoding

Prolly values use a versioned binary envelope around canonical JSON bytes. The canonical encoder:

1. Sorts object keys by UTF-8 byte order recursively.
2. Emits integers as minimal base-10 text without leading zeroes.
3. Emits finite floating-point values with deterministic shortest round-trip formatting,
   while retaining floating-point identity (`1.0` does not canonicalize to integer `1`).
4. Retains negative zero as floating-point negative zero when accepted by the parser.
5. Preserves array order unless a declared Compass field has set semantics.
6. Rejects non-finite numeric input before serialization.
7. Emits UTF-8 without insignificant whitespace.
8. Tags every record with its schema name and version.

Canonicalization applies to content identity, not to user-facing formatting. Reconstruction uses the stored order and placement metadata.

Golden byte fixtures cover integer boundaries, exponent forms, `1` versus `1.0`, negative
zero, Unicode escaping, and recursive object ordering. The canonical-encoding version is
part of `compass/store-format/v1`; changing any golden byte requires an explicit migration
decision.

## Exact-commit materialization

### Repository and revision rules

Repository discovery resolves the top level and Git common directory through Git commands. Revision resolution accepts a user revision, passes `--end-of-options`, resolves it to one full commit ID, and stores lowercase SHA-1 or SHA-256 text.

The builder creates a detached temporary worktree below `<git-common-dir>/compass/tmp`. It verifies that the worktree `HEAD` equals the requested full commit before extraction. It never modifies or reads uncommitted files from the caller's checkout.

Before checkout, Compass inventories configured filter drivers. It rejects unsupported
external smudge/process drivers with a typed limitation instead of executing them. The
worktree is then created with checkout hooks disabled, `GIT_LFS_SKIP_SMUDGE=1`, credential
prompts disabled, and no command that fetches missing objects. Compass preserves committed
LFS pointer files and reports LFS pointers and gitlinks. Missing local Git objects fail
materialization.

Historical detection applies `.gitignore` and `.graphifyignore` files present in the target
tree plus explicit normalized profile excludes. It deliberately ignores `.git/info/exclude`,
global excludes, and caller-worktree ignore state. Every applied committed ignore file and
explicit exclude enters the extraction fingerprint.

V1 does not fetch submodules or LFS objects. It extracts the committed superproject representation and reports detected gitlinks and LFS pointers. Network-dependent recursive materialization requires a separate opt-in design.

### Ancestor seeding

Ancestor reuse is an optimization. Walk the target's first-parent history and select the nearest validated preferred realization with the same extraction fingerprint. Reconstruct its graph and extraction manifest into the isolated output directory before running incremental extraction.

If no compatible ancestor exists, perform a full build. The target commit's checked-out tree remains authoritative for linear, branch, and merge commits.

### Complete builder boundary

The reusable materializer accepts an injected builder:

```rust
trait CompleteGraphBuilder {
    fn build(&self, checkout: &Path, output: &Path,
        seed: Option<&GraphArtifacts>)
        -> Result<CompletedGraphArtifacts, MaterializeError>;
}
```

`CompletedGraphArtifacts` contains `GraphArtifacts` and `CompletionEvidence`. The production CLI builder runs the normal Compass pipeline with partial mode disabled, verifies `built_at_commit`, and derives semantic counts from the completed build and manifest. It never infers success from token counts.

## Eager and lazy lifecycle

### Eager post-commit generation

`compass history enable` stores a normalized, non-secret eager build profile in
`config.json`, creates/verifies the store, and installs or updates the existing managed
post-commit hook block while preserving user-owned hook content. Enablement rolls back if the
managed block cannot be installed. `disable` clears that enabled state without
deleting the database, realizations, jobs, or leases and without terminating a worker that
already owns a live lease; the now-inert managed block remains installed. Explicit historical commands continue to work while eager history
is disabled. Re-enabling replaces the stored profile only after validating all requested
options.

When history is enabled, the managed post-commit hook resolves the new full commit ID, atomically enqueues it, starts
a detached queue-draining worker, and returns. History enqueueing occurs before refresh-only guards
for rebase, merge, cherry-pick, changed-file filtering, or linked worktrees. Those guards may
suppress rebuilding the current `graphify-out`, but they never suppress commit-history
enqueueing. A launch failure leaves the queued job for the next worker launch; an explicit
build/query of that same commit may also claim and complete its durable attempt synchronously.

The worker reconciles stale attempts and drains every queued job in FIFO order before exiting.
Multiple workers safely join or skip leases, and one terminally failed job does not prevent
later queued jobs from running. For each claimed job it:

1. Claims or joins the job lease.
2. Creates an exact detached worktree.
3. Selects a compatible first-parent seed.
4. Runs complete extraction, inference, clustering, and analysis.
5. Builds and validates the typed roots.
6. Publishes the realization and attempts the preferred compare-and-swap.
7. Removes the worktree and records the terminal job result.

The hook captures the commit before returning. Later branch movement, amend, rebase, or checkout operations do not change the job target.

### Lazy historical generation

Query, path, explain, and both sides of diff use one `resolve_or_materialize` service. When a commit has no preferred realization, the command joins an existing compatible job or runs the build synchronously and reports stages on stderr.

Lazy commands preserve machine-readable stdout. A provider, Git, validation, or store error returns a nonzero exit status and leaves the previous catalog unchanged.

## Jobs, leases, and crash recovery

Operational state remains outside the immutable Prolly store:

```text
jobs/<job-id>.json
leases/<commit>-<profile-digest>.lease
locks/maintenance.lock
```

Job files use bounded canonical JSON and crash-durable same-directory replacement: write a new owner-only temporary file, flush and synchronize it, atomically replace the destination, and synchronize the parent directory. Unix uses atomic `rename`; Windows uses a replace-existing, write-through primitive. Unsupported filesystems fail closed rather than claiming durability. They contain the commit, normalized non-secret build profile, profile digest, optional resolved fingerprint, state, attempt count, diagnostic, realization ID, preferred result, timestamps, and lease generation. A job file is limited to 1 MiB and its redacted diagnostic to 64 KiB.

The state machine is:

```text
queued -> building -> validating -> published
                 \-> failed
                 \-> incomplete
```

`published` records `preferred: true` or `preferred: false`. `incomplete` never records a cataloged realization.

A queued job cannot claim a final extraction fingerprint before Compass reads configuration and ignore inputs from the exact target commit. The queue therefore deduplicates by commit and normalized build-profile digest. After creating the detached worktree, the worker resolves the full fingerprint, stores it in the job, and validates that its binary and pipeline versions match any previous attempt.

A worker claim creates or compare-and-swaps a lease containing a random owner ID, generation, and expiration. Leases last 120 seconds and refresh every 30 seconds during long extraction stages. A new worker may reclaim an expired lease by incrementing its generation. Every state transition and terminal write checks the generation, so late writes from an old generation fail. Wall-clock jumps can cause redundant work, but cannot corrupt the catalog or overwrite a newer job generation.

Before catalog publication, the worker persists the candidate realization ID and the preferred ID it observed. Recovery reconciles stale jobs against the immutable catalog. If the realization exists, recovery retries the preferred compare-and-swap only when the observed preferred ID still matches; otherwise it records a published non-preferred result. This covers termination between catalog publication, preferred selection, and terminal job persistence.

Lazy and eager requests for the same commit and build profile join one live lease. Explicit `compass history rebuild` creates a new attempt and may produce a distinct realization. Credentials and credential values never enter jobs, leases, diagnostics, fingerprints, or manifests.

## Activity and maintenance locking

SQLite protects database transactions, but it cannot protect a Prolly builder's
not-yet-published nodes from concurrent garbage collection. Compass therefore uses one
cross-process reader-writer lock file, `locks/maintenance.lock`:

- query, diff, reconstruction, publication, and active builders hold a shared activity lock
- garbage collection, non-preferred pruning, database replacement, and future migrations hold the exclusive maintenance lock

The lock spans the complete multi-read or multi-write operation. This prevents garbage collection from deleting nodes used by an active reader or builder. SQLite root compare-and-swap still resolves ordinary publication contention; the maintenance lock is not a global writer lock.

The implementation uses Rust's standard `File::{try_lock_shared,try_lock,unlock}` APIs and a
bounded deadline loop; it does not add a second locking crate. Acquisition is bounded and interruptible. Guards release locks through RAII and operating-system
process teardown. Lock upgrades are forbidden: a command must release a shared guard before
requesting exclusive maintenance, then revalidate all observed state. Every command acquires
the maintenance lock before opening a database transaction, preventing lock-order deadlocks.
There is no separate `activity.lock`; two independent lock files would not provide mutual
exclusion.

## Query and reconstruction

Graph-reading commands accept one exclusive source selector:

```text
compass query "authentication flow" --at <revision>
compass path <source> <target> --at <revision>
compass explain <symbol> --at <revision>
```

The loader resolves or materializes the preferred realization, validates it, reconstructs a `GraphDocument` in memory, and creates the existing immutable query graph. It does not write temporary `graph.json`.

Historical queries use an empty learning overlay. The current checkout's `.graphify_learning.json` is experiential state, not committed realization data.

Compatibility export remains explicit:

```text
compass history export <revision> --format graph-json --output <path>
compass history export <revision> --format graphify-out --output <directory>
```

The bundle export writes stored authoritative files verbatim and regenerates every registered
derived artifact with the renderer version recorded by the realization. It fails before
publishing the destination when a required renderer version is unavailable; it never silently
uses the current renderer and claims historical equivalence. The bundle destination must not
already exist; v1 performs no implicit merge or destructive force-replacement.

Reconstruction restores node and edge order, legacy `edges` spelling, hyperedge placement, unknown fields, multigraph discriminators, analysis, labels, manifest data, and semantic metadata. Tests compare parsed `GraphDocument` and sidecars, then verify stable serialized output.

## Streaming graph-aware diff

```text
compass diff <old-revision> <new-revision>
```

Both revisions resolve or lazily materialize preferred realizations. Equal tree CIDs skip a record kind. Prolly's streaming diff skips equal subtrees and emits ordered changes without reconstructing both graphs.

Compass projects records into:

- added, removed, and changed nodes
- added, removed, and changed edges
- added, removed, and changed hyperedges with stable IDs
- removal and addition for synthetic-key record changes
- community, label, score, and ranked-analysis changes
- graph metadata and extraction-configuration changes

Output modes remain:

```text
compass diff A B
compass diff A B --detailed
compass diff A B --format json
compass diff A B --topology-only
```

Summary mode retains bounded examples. Detailed mode and JSON mode write one change at a time. A failing output sink stops traversal and returns an error.

## Validation and corruption handling

Publication and explicit audit validate:

- store-format compatibility
- every named root and persisted tree format
- recomputed manifest digest and realization ID
- all five tree counts against manifest counts
- typed-key schema and decoded record identity
- unique node IDs and valid multigraph discriminators
- edge endpoints and hyperedge members
- analysis references and metadata order records
- commit and fingerprint against catalog location
- canonical envelope round-trip
- completion evidence with no partial or failed semantic work
- the v1 resource limits: 512 MiB total authoritative input, 1 MiB encoded key, 64 MiB encoded record value, 10,000,000 records per tree, JSON nesting depth 128, 1 MiB job record, and 64 KiB redacted diagnostic

Unknown schema versions fail with a compatibility error. Corrupt preferred realizations never
trigger automatic fallback or ordinary replacement. A normal eager build, lazy build, or
rebuild may publish a valid candidate, but it remains non-preferred while the corrupt preferred
root exists. Replacement requires:

```text
compass history rebuild <revision> --replace-corrupt
```

Recovery validates the candidate, records the corrupt preferred ID in attempt history, and
compare-and-swaps from that exact preferred value. Concurrent repair or preferred selection
produces a conflict instead of overwriting the winner.

## Garbage collection and retention

Normal retention keeps every published realization, including non-preferred ones. Git history rewriting does not automatically remove graph history.

Under the exclusive maintenance lock, normal garbage collection:

1. Snapshots all retained named roots.
2. Uses Prolly retention planning to trace reachable CIDs.
3. Rechecks the catalog digest before deletion.
4. Deletes only unreachable node rows.
5. Removes terminal attempt records older than 30 days and temporary directories older than
   24 hours only when neither has a live lease. Queued or active attempts are never removed.

`--prune-non-preferred` first lists candidate realization IDs and requires explicit confirmation. Pruning removes their six named roots in a strict transaction, recomputes reachability, and then deletes unreachable nodes. A preferred change or catalog digest mismatch invalidates the plan without deleting anything.

SQLite may keep freed pages inside `history.sqlite`; later writes reuse them. V1 reports reclaimed rows and reusable bytes when available, but it does not claim physical file shrinkage.

## Commands

V1 exposes this canonical Compass surface:

```text
compass history enable [build-profile options]
compass history disable
compass history status [<revision>] [--format text|json]
compass history build <revision> [build-profile options] [--format text|json]
compass history rebuild <revision> [build-profile options] [--replace-corrupt] [--format text|json]
compass history list [<revision>] [--format text|json]
compass history show <realization-id> [--format text|json]
compass history prefer <revision> <realization-id> [--format text|json]
compass history export <revision> --format graph-json|graphify-out --output <path>
compass history gc [--prune-non-preferred] [--yes] [--format text|json]
compass diff <old-revision> <new-revision> [--detailed|--format json] [--topology-only]
```

Only the `compass` forms above are part of the versioned-graph public contract. Existing legacy
`graphify` forwarding may continue on a best-effort basis, but missing commands or differences in
help, stdout, stderr, exit status, and side effects are not Compass regressions. Compass help and
diagnostics always use the `compass` spelling.

`compass history status` reports enablement, preferred validation, queued or active jobs, failed
attempts, gitlink or LFS limitations, and store compatibility. On a repository that has never
created history it reports `disabled`/`no store` and exits successfully without creating files.
`disable` is idempotent. `list` on an absent or empty store is an empty success. Explicit
build/rebuild and lazy `--at` materialization are permitted while eager history is disabled.
Status still renders its report but exits `1` when an existing selected preferred realization
is corrupt or the store format is incompatible; an absent store is not an error.

Exit code `0` means success, including empty read-only results; `2` means command-line usage
error; `1` means repository, Git, provider, validation, corruption, lock, output, or store
failure. Progress and human diagnostics go to stderr. Text or JSON results go to stdout, and
JSON output is a single valid value even for empty results. Broken output pipes stop work and
return failure without corrupting store state. Parsers reject unknown/repeated singleton
options and honor `--` as the end of options for revision and path operands.

## Testing strategy

### Canonical and schema tests

- Object insertion order produces identical canonical bytes and tree roots
- Typed keys remain collision-free for Unicode, NUL bytes, and separators
- Directed, undirected, simple, and multigraph edge identities match the declared rules
- Exact duplicate multigraph edges survive reconstruction
- Exact duplicate id-less hyperedges survive reconstruction
- Node, edge, hyperedge, unknown-field, sidecar, and order data round-trip
- Canonical number golden fixtures remain byte-stable across reopen and dependency checks
- Artifact registry classification rejects undeclared authoritative sidecars
- Derived reports regenerate with canonical semantic equivalence
- Raw token, cost, timing, and diagnostic variation does not change realization identity
- Fingerprints include every declared meaning input and exclude secrets and runtime-only data
- Manifest identity excludes operational values

### SQLite adapter and durability tests

- Compile-time assertions cover every required Prolly store trait
- In-memory and file-backed databases pass publication tests
- Named roots and trees survive close and reopen
- Strict multi-root transactions commit or roll back together
- Concurrent connections exercise WAL, busy timeout, and preferred conflicts
- Abrupt worker termination leaves the previous preferred realization readable
- Store-format mismatch and corrupt root data fail closed
- Unix permission tests verify owner-only defaults
- operational-file fault injection proves temp sync, atomic replacement, parent sync, and symlink rejection
- Old fixture databases reopen under the exact pinned dependencies

### Property and differential tests

- Random insertion order yields identical roots
- Random graph edits reconstruct with exact supported structure and ordering
- Streaming Prolly diff equals full in-memory graph comparison
- Historical query, path, and explain equal `graph.json` behavior with an empty overlay
- SQLite-backed and in-memory test stores produce identical logical roots

### Git, job, and maintenance scenarios

- linear history, branches, merge commits, renames, and deletions
- SHA-1 and SHA-256 repositories
- shallow history and missing Git objects
- linked worktrees sharing one database
- uncommitted caller files excluded from historical builds
- `.git/info/exclude` and global ignores do not affect historical fingerprints or corpora
- checkout hooks, LFS smudging, credential prompts, and network fetching remain disabled
- detected gitlinks and LFS pointers reported without network access
- eager queueing, lazy joining, stale-lease recovery, and late-worker rejection
- disabled hooks enqueue nothing; enable installs the managed block; disable retains queryable history
- one worker drains all queued attempts in FIFO claim order and continues after terminal failures
- no-store status/list create no files and Compass process contracts remain stable
- provider failure, partial output, retry, and recovery
- same-fingerprint nondeterministic realizations and preferred contention
- garbage collection racing an active reader or builder
- shared and exclusive operations contend on the same maintenance lock file
- stale prune plans and interrupted maintenance
- corrupt preferred replacement requires `--replace-corrupt` and an exact compare-and-swap

### Performance qualification

Benchmarks record:

- cold and seeded materialization time
- database growth and reusable pages
- publication contention across processes
- diff latency and peak buffered records
- query latency and peak memory
- garbage-collection planning and sweep time

Default enablement requires measurable subtree reuse and diff benefit without a material query regression. The design does not invent a percentage threshold before measurements exist.

## Rollout

### Phase 1: immutable SQLite storage

- Add `compass-history` and exact Prolly dependency pins
- Implement SQLite opening, format verification, canonical encoding, typed keys, manifests, publication, validation, reconstruction, and streaming diff
- Add adapter, schema, property, corruption, and reopen tests

### Phase 2: explicit history commands

- Add the Compass build, rebuild, list, show, prefer, export, diff, and garbage-collection commands
- Keep current graph persistence as the default

### Phase 3: historical query and lazy materialization

- Add `--at` to query, path, and explain
- Add exact detached worktrees, compatible ancestor seeding, and synchronous lazy builds

### Phase 4: eager generation and recovery

- Add explicit enable/disable lifecycle and extend the managed post-commit hook
- Add durable jobs, leases, worker recovery, retries, and diagnostics
- Qualify linked-worktree and concurrent-process behavior

### Phase 5: default-persistence evaluation

- Run semantic-correctness, durability, and performance qualification
- Make history the default only when every gate passes
- Keep `graph.json` as a compatibility export
- Design remote synchronization and mutable-source snapshots separately

## Acceptance criteria

The feature is complete when:

1. Rust commands materialize and query any available Git commit from its exact superproject tree.
2. Every preferred realization contains complete AST, semantic, inferred, hyperedge, clustering, and analysis data.
3. SQLite transactions and catalog compare-and-swap preserve all published concurrent realizations.
4. Reconstruction preserves Compass's supported simple and multigraph structure, order, sidecars, and unknown attributes.
5. Cross-version diff streams Prolly changes without loading both complete graphs.
6. Post-commit work is durable and does not block on extraction.
7. Lazy backfill shares the same complete materializer and joins compatible active work.
8. Interrupted, incomplete, corrupt, and unsupported builds cannot become preferred.
9. Linked worktrees share one owner-protected SQLite store safely.
10. Garbage collection cannot race active readers, builders, or publication.
11. Existing commands behave as before when history is disabled.
12. Exact pinned adapter versions pass reopen, transaction, concurrency, and migration fixtures.
13. Artifact export preserves canonical semantic equivalence while keeping operational attempt data outside realization identity.
14. Historical materialization is independent of caller-local ignore files, checkout hooks, and network-dependent filters.
15. Read-only no-store commands create nothing, and eager enqueue occurs only while explicitly enabled.
16. Compass is the sole canonical frontend; its help, parsing, output, exit status, and side effects pass the documented process contracts without depending on Graphify alias behavior.
17. Durable job writes, generation leases, worker draining, retention, and path validation pass fault and concurrency tests.

## Audit conclusions and residual risks

This revision resolves the original design's storage-layout mismatch, ambiguous publication atomicity, unproven completeness marker, multigraph contradiction, ordering ambiguity, job-recovery gap, garbage-collection races, accidental eager opt-in, frontend ambiguity, non-durable operational writes, unbounded validation policy, and unspecified retention.

No design can guarantee defect-free implementation. These risks remain measurable rather than hidden:

- SQLite permits one writer at a time, so publication contention must pass workload benchmarks
- semantic providers can return nondeterministic complete results, so multiple realizations remain necessary
- bundled SQLite increases binary size and requires platform qualification
- submodule and LFS contents remain outside v1's exact superproject snapshot
- deleted SQLite pages may remain allocated until later reuse
- future Prolly or adapter upgrades require explicit format and reopen qualification
- old derived artifacts can be regenerated only while their recorded renderer version remains supported
- wall-clock jumps may duplicate leased work, although generation checks prevent stale catalog/job writes

The implementation plan must preserve these invariants and attach a failing test to every acceptance criterion before changing production behavior.
