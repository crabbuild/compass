# Compass Versioned Graph Prolly Tree Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Revised:** 2026-07-22 after the Compass workspace extraction and flat-command rename

**Governing designs:** `docs/superpowers/specs/2026-07-19-compass-native-rust-port-design.md` and `docs/superpowers/specs/2026-07-21-versioned-graph-prolly-tree-design.md`

**Goal:** Persist one complete, immutable Compass realization per Git commit in typed Prolly trees, then support historical query and graph-aware diff with opt-in eager and on-demand lazy materialization through a fully usable Compass-first command surface.

**Architecture:** A new `compass-history` crate converts completed `GraphDocument` artifacts into five immutable Prolly trees and publishes their roots through a content-addressed `GraphVersion` manifest. `prolly-store-sqlite` is the only v1 node/root backend, shared by linked worktrees through the Git common directory. Git revision/worktree orchestration remains above the storage layer, while CLI commands resolve commits, materialize missing versions, and load historical graphs into the existing `compass-query` path.

**Tech Stack:** Rust 2024, Rust 1.97.1 toolchain with workspace MSRV 1.97, `prolly-map = "=0.5.0"`, `prolly-store-sqlite = "=0.3.0"`, Serde/JSON, SHA-256, SQLite WAL, existing Compass graph/core/query crates, and Git CLI.

## Global Constraints

- Implement inside the standalone Compass Git repository at `/Users/haipingfu/graphify/compass`; it is mounted as the Graphify superproject's `compass` submodule, so code commits and worktrees belong to the Compass repository rather than the superproject.
- File lists use `compass/...` paths relative to the Graphify superproject for unambiguous location. Every shell, Cargo, and Git snippet assumes its working directory is the Compass repository or isolated Compass worktree, so command operands omit the outer `compass/` prefix.
- Use exactly `prolly-map = "=0.5.0"` and `prolly-store-sqlite = "=0.3.0"`; the library crate names are `prolly` and `prolly_store_sqlite`.
- Store history below the path returned by `git rev-parse --git-common-dir`, never in the working tree.
- Store Prolly data at `<git-common-dir>/compass/history.sqlite`; never read or write the adapter's private SQL tables directly.
- Open SQLite with WAL enabled, `synchronous_normal: false`, and a 10-second busy timeout.
- Persist and verify `compass/store-format/v1` before accessing application roots.
- A preferred realization must contain the complete graph, including semantic and inferred relationships; partial semantic results never enter the version catalog.
- The realization identity excludes timestamps, machine paths, credentials, and runtime-only Prolly cache settings.
- Preserve every supported `GraphDocument` field and unknown JSON attribute through storage and reconstruction.
- Schema v1 preserves directed, undirected, simple, and multigraph documents, including exact duplicate parallel edges and exact duplicate id-less hyperedges.
- Preserve node, edge, and hyperedge order, legacy `edges`, original hyperedge placement, unknown values, and authoritative sidecars.
- Historical compatibility means canonical semantic equivalence; derived reports and HTML are regenerated rather than stored byte-for-byte.
- Raw token use, cost, timings, diagnostics, caches, and learning overlays do not enter realization identity.
- Historical builds apply committed `.gitignore`/`.graphifyignore` files plus explicit profile excludes, never `.git/info/exclude` or global excludes.
- Temporary worktrees disable hooks, LFS smudging, credential prompts, network fetching, and unsupported external filters.
- Use one `locks/maintenance.lock`, shared by readers/builders/publication and exclusive to GC/pruning/migration; lock upgrades are forbidden.
- Replacing a corrupt preferred realization requires explicit `compass history rebuild <rev> --replace-corrupt` recovery.
- Keep `graph.json` behavior compatible while history is opt-in.
- Treat `compass` as the sole canonical Rust command surface. A legacy `graphify` binary may remain as a best-effort transition shim, but its command availability and behavior do not constrain Compass or this plan's acceptance gates.
- Every canonical command begins directly with `compass`; never reintroduce the removed legacy binary or a nested graph-command namespace.
- Eager history begins only after `compass history enable`; `compass history status`, `compass history list`, and explicit `compass history build`, `compass query`, or `compass diff` commands must not silently enable it. Disabling eager history never deletes stored history.
- Read-only commands open an existing store without creating the history directory or SQLite database. Explicit enablement or materialization may create it.
- Use v1 limits of 512 MiB total authoritative input, 1 MiB key, 64 MiB record value, 10,000,000 records per tree, JSON depth 128, 1 MiB job record, and 64 KiB diagnostic.
- Do not add remote synchronization, per-file roots, or new graph semantics in this implementation.
- Use Prolly named roots and strict transactions for atomic multi-root publication.
- Preserve unrelated working-tree changes. Start execution from the Compass repository in an isolated Compass worktree with `superpowers:using-git-worktrees`; never use a Graphify-superproject worktree as a substitute.
- Follow red-green-refactor for every behavior change and use small commits at the end of each task.
- Retain established Graphify-compatibility resources exactly as the current Compass code defines them: `graphify-out/`, `graph.json`, `.graphifyignore`, `.graphify_*` sidecars, and `GRAPHIFY_*` environment variables. They are artifact/API compatibility names, not stale workspace branding.
- Run `compass update .` after the implementation changes are complete so `graphify-out/` matches the resulting Compass code.

The naming boundary is mechanical and testable:

| Concern | Required name |
|---|---|
| Repository directory | `compass/` in the Graphify superproject |
| Crates and Rust modules | `compass-*` and `compass_*` |
| Canonical executable/command prefix | `compass` with flat subcommands |
| History directory/database | `<git-common-dir>/compass/history.sqlite` |
| Prolly store-format and named-root namespace | `compass/store-format/v1` and `compass/v1/...` |
| Legacy executable | `graphify`, optional best-effort transition shim outside the Compass contract |
| Compatibility artifacts/configuration | `graphify-out/`, `graph.json`, `.graphifyignore`, `.graphify_*`, and `GRAPHIFY_*`, retained verbatim |

---

## File and crate map

Create `compass/crates/compass-history/` with these responsibilities:

```text
compass-history/
├── Cargo.toml
├── src/
│   ├── lib.rs          public API and re-exports
│   ├── error.rs        typed storage, schema, Git, and validation failures
│   ├── canonical.rs    canonical JSON and schema envelopes
│   ├── keys.rs         segment-safe typed Prolly and named-root keys
│   ├── fingerprint.rs  extraction-fingerprint inputs and digest
│   ├── model.rs        GraphVersion, RealizationId, StoredTree, catalog types
│   ├── artifacts.rs    graph/sidecar decomposition and reconstruction
│   ├── store.rs        SQLite adapter, store format, publication, reads, listing
│   ├── validate.rs     root, count, endpoint, and schema validation
│   ├── diff.rs         streaming typed-tree graph diffs
│   ├── git.rs          revision resolution, common-dir discovery, worktree guard
│   ├── config.rs       opt-in eager-history profile and enablement state
│   ├── jobs.rs         durable eager-build queue and state transitions
│   ├── durable.rs      crash-durable owner-only operational-file replacement
│   ├── leases.rs       generation leases, heartbeat, stale-worker rejection
│   ├── lock.rs         shared activity and exclusive maintenance guards
│   └── gc.rs           reachability planning and explicit sweeping
└── tests/
    ├── canonical.rs
    ├── roundtrip.rs
    ├── publication.rs
    ├── diff.rs
    ├── git.rs
    ├── jobs.rs
    └── maintenance.rs
```

Modify existing crates as follows:

- `compass/crates/compass-core/src/history.rs`: materialization orchestration that invokes a supplied complete-graph builder and publishes its artifacts.
- `compass/crates/compass-core/src/lib.rs`: export history orchestration and construct `LoadedGraph` from a reconstructed document.
- `compass/crates/compass-cli/src/history_commands.rs`: `history` command family, `diff`, lazy materialization, export, and GC presentation.
- `compass/crates/compass-cli/src/history_build.rs`: current-executable complete-build adapter used inside detached worktrees.
- `compass/crates/compass-cli/src/lib.rs`: command dispatch, `--at` parsing, graph-source selection, and help.
- `compass/crates/compass-cli/src/hook_commands.rs`: enqueue the committed SHA and launch the history worker without blocking the commit.
- `compass/crates/compass-cli/tests/history_cli.rs`: end-to-end CLI history, query, diff, and lazy-build coverage.
- `compass/crates/compass-cli/tests/hook_cli.rs`: eager history hook assertions.
- `compass/crates/compass-output/src/history_bundle.rs`: version-dispatched regeneration of derived history artifacts without a dependency on `compass-history`.
- `compass/README.md`: Compass-first history command documentation and local-store behavior.

## Shared interfaces

Later tasks use these exact public types:

```rust
pub struct HistoryStore;

pub struct PublishRequest {
    pub commit: CommitId,
    pub parents: Vec<CommitId>,
    pub fingerprint: ExtractionFingerprint,
    pub artifacts: GraphArtifacts,
    pub completion: CompletionEvidence,
    pub make_preferred: bool,
}

pub struct PublishedVersion {
    pub id: RealizationId,
    pub version: GraphVersion,
    pub preferred: bool,
}

pub struct GraphArtifacts {
    pub document: GraphDocument,
    pub analysis: Option<serde_json::Value>,
    pub labels: Option<serde_json::Value>,
    pub manifest: Option<serde_json::Value>,
    pub authoritative_sidecars: std::collections::BTreeMap<String, ArtifactContent>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CompletionEvidence {
    pub extraction_succeeded: bool,
    pub allow_partial: bool,
    pub semantic_files_expected: u64,
    pub semantic_files_completed: u64,
    pub failed_chunks: u64,
}

pub struct CompletedGraphArtifacts {
    pub artifacts: GraphArtifacts,
    pub completion: CompletionEvidence,
}

pub struct MaterializeRequest {
    pub repository: Repository,
    pub commit: CommitId,
    pub profile: BuildProfile,
    pub rebuild: bool,
    pub replace_corrupt: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializeStage {
    Building,
    Validating,
    Publishing,
}

pub trait MaterializeObserver {
    fn entered(&mut self, stage: MaterializeStage) -> Result<(), MaterializeError>;
}

pub trait CompleteGraphBuilder {
    fn build(
        &self,
        checkout: &Path,
        output_root: &Path,
        seed: Option<&GraphArtifacts>,
    ) -> Result<CompletedGraphArtifacts, MaterializeError>;
}
```

Method signatures remain consistent across tasks: `HistoryStore::preferred` takes
`&CommitId`; `CommitId` exposes `as_str` and implements `Display`; fingerprints and
realization IDs expose lowercase hexadecimal text and strict parsing. CLI code never
reaches into tuple-struct internals.

Store lifecycle is explicit throughout the plan:

```rust
impl HistoryStore {
    pub fn create(repository: &Repository) -> Result<Self, HistoryError>;
    pub fn open_existing(repository: &Repository)
        -> Result<Option<Self>, HistoryError>;
}
```

`create` is reserved for `compass history enable` and explicit/lazy materialization. Read-only
commands use `open_existing`; absence is a normal state rather than permission to mutate the
repository. The existence of an SQLite store does not mean eager history is enabled.

## Public CLI and process contract

The commands below are specified only for `compass`. Existing `graphify` forwarding may remain,
but it is not required to expose the same commands or reproduce Compass help, output, status, or
side effects. Compass help and errors always use the `compass` spelling.

```text
compass history enable [build-profile options]
compass history disable
compass history status [REV] [--format text|json]
compass history build REV [build-profile options] [--format text|json]
compass history rebuild REV [build-profile options] [--replace-corrupt] [--format text|json]
compass history list [REV] [--format text|json]
compass history show REALIZATION [--format text|json]
compass history prefer REV REALIZATION [--format text|json]
compass history export REV --format graph-json|graphify-out --output PATH
compass history gc [--prune-non-preferred] [--yes] [--format text|json]
compass diff OLD NEW [--detailed|--format json] [--topology-only]
compass query QUERY --at REV
compass path SOURCE TARGET --at REV
compass explain SYMBOL --at REV
```

Exit `0` means success, including idempotent disable, no-store status, and an empty list. Exit
`2` is reserved for usage errors. Repository, Git, provider, validation, corruption, lock,
output, and storage errors exit `1`. Results go to stdout; progress and diagnostics go to
stderr. A JSON mode emits exactly one valid JSON value, including empty results. A broken
output sink aborts streaming promptly without changing an already valid preferred realization.
Every parser rejects unknown or repeated singleton options, supports `--flag=value` where the
existing CLI does, and honors `--` as the end of options so revision and path operands cannot
be mistaken for flags.

Test-only fixture helpers shown below are local to their named integration-test file.
Each helper constructs the smallest committed Git repository and/or `GraphArtifacts`
needed by that test, returns a `tempfile::TempDir`-owning fixture so paths stay alive,
and never invokes a provider unless the test explicitly covers provider failure.
`version_one`/`version_two` differ by one node, one edge, and one analysis value;
`fixture_publish_request` always returns complete cross-checked `CompletionEvidence`. This contract is
implemented when the first test using each helper is added, rather than as production API.

### Task 1: Scaffold `compass-history` and prove the SQLite Prolly contract

**Files:**
- Modify: `compass/Cargo.toml`
- Modify: `compass/Cargo.lock`
- Create: `compass/crates/compass-history/Cargo.toml`
- Create: `compass/crates/compass-history/src/lib.rs`
- Create: `compass/crates/compass-history/src/error.rs`
- Create: `compass/crates/compass-history/src/git.rs`
- Create: `compass/crates/compass-history/src/lock.rs`
- Create: `compass/crates/compass-history/src/store.rs`
- Create: `compass/crates/compass-history/tests/sqlite_contract.rs`

**Interfaces:**
- Consumes: `prolly::{Store, ManifestStore, ManifestStoreScan, NodeStoreScan, TransactionalStore}` and `prolly_store_sqlite::{SqliteStore, SqliteStoreConfig}`.
- Produces: `HistoryStore::{create, open_existing}`, `HistoryStore::database_path`, `ActivityGuard`, and `MaintenanceGuard`.

- [ ] **Step 1: Write the failing storage contract test**

```rust
use prolly::{ManifestStore, ManifestStoreScan, NodeStoreScan, TransactionalStore};
use compass_history::HistoryStore;

fn requires_store_contract<T>()
where
    T: ManifestStore + ManifestStoreScan + NodeStoreScan + TransactionalStore,
{
}

#[test]
fn sqlite_store_has_every_publication_capability() -> Result<(), Box<dyn std::error::Error>> {
    requires_store_contract::<prolly_store_sqlite::SqliteStore>();
    let fixture = committed_repository()?;
    let repository = Repository::discover(fixture.path())?;
    assert!(HistoryStore::open_existing(&repository)?.is_none());
    let history = HistoryStore::create(&repository)?;
    assert!(HistoryStore::open_existing(&repository)?.is_some());
    assert_eq!(history.database_path(), repository.common_dir().join("compass/history.sqlite"));
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify the crate is missing**

Run: `cargo test -p compass-history --test sqlite_contract sqlite_store_has_every_publication_capability`

Expected: FAIL because workspace package `compass-history` does not exist.

- [ ] **Step 3: Add the workspace member and pinned dependency**

Add `"crates/compass-history"` to `[workspace].members` and add both exact pins:

```toml
[workspace.dependencies]
prolly-map = "=0.5.0"
prolly-store-sqlite = "=0.3.0"
```

Create the crate manifest:

```toml
[package]
name = "compass-history"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
readme.workspace = true
description.workspace = true
keywords.workspace = true
categories.workspace = true

[dependencies]
prolly-map.workspace = true
prolly-store-sqlite.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true
compass-files = { path = "../compass-files", version = "0.1.0" }
compass-model = { path = "../compass-model", version = "0.1.0" }

[dev-dependencies]
tempfile.workspace = true

[lints]
workspace = true
```

- [ ] **Step 4: Implement the smallest durable store wrapper**

In `git.rs`, implement `Repository::discover` using `git rev-parse --show-toplevel` and
`git rev-parse --git-common-dir`, resolving relative common-dir output against the top level.
Task 7 extends this same type with revision and parent operations.

`HistoryStore::create` creates the owner-only directory and `locks/maintenance.lock` before opening SQLite. A new
database acquires the exclusive maintenance guard through format initialization; an existing
database acquires the shared guard through format verification. Release the opening guard
only after the adapter and `compass/store-format/v1` are consistent. `open_existing` first
checks the validated nonsymlink path and returns `Ok(None)` without creating any path when the
database is absent; when present it performs the same shared-guard verification.

```rust
// src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("could not open history store: {0}")]
    Store(#[from] prolly_store_sqlite::SqliteStoreError),
    #[error("prolly operation failed: {0}")]
    Prolly(#[from] prolly::Error),
}
```

```rust
// src/store.rs
use std::path::{Path, PathBuf};
use std::sync::Arc;

use prolly::{Config, Prolly};
use prolly_store_sqlite::{SqliteStore, SqliteStoreConfig};

use crate::{HistoryError, Repository};

const STORE_FORMAT_ROOT: &[u8] = b"compass/store-format/v1";

pub struct HistoryStore {
    root: PathBuf,
    database_path: PathBuf,
    pub(crate) prolly: Prolly<Arc<SqliteStore>>,
}

impl HistoryStore {
    pub fn create(repository: &Repository) -> Result<Self, HistoryError> {
        let root = repository.common_dir().join("compass");
        create_owner_only_history_dir(&root)?;
        let database_path = root.join("history.sqlite");
        let backend = Arc::new(SqliteStore::open_with_config(
            &database_path,
            SqliteStoreConfig {
                busy_timeout_ms: 10_000,
                enable_wal: true,
                synchronous_normal: false,
            },
        )?);
        let store = Self {
            root,
            database_path,
            prolly: Prolly::new(backend, Config::default()),
        };
        store.initialize_or_verify_store_format(STORE_FORMAT_ROOT)?;
        Ok(store)
    }

    pub fn open_existing(repository: &Repository) -> Result<Option<Self>, HistoryError> {
        let paths = HistoryPaths::existing(repository)?;
        let Some(paths) = paths else { return Ok(None) };
        Self::open_verified(paths).map(Some)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }
}
```

```rust
// src/lib.rs
mod error;
mod lock;
mod git;
mod store;

pub use error::HistoryError;
pub use lock::{ActivityGuard, MaintenanceGuard};
pub use git::Repository;
pub use store::HistoryStore;
```

- [ ] **Step 5: Run focused checks**

Before the focused checks, implement `create_owner_only_history_dir` with `0700` Unix directory mode and `0600` database/operational files, using the closest owner-only platform controls elsewhere. Reject symlinks at the `compass` directory, database, lock, configuration, job, lease, and temp targets. Add tests for absent read-only open causing no filesystem changes, concurrent first-open format initialization, unknown store-format rejection, close/reopen, strict rollback, WAL and busy-timeout contention, owner-only Unix permissions, symlink rejection, and reopening a checked-in v1 fixture database. `initialize_or_verify_store_format` must use adapter root CAS rather than private SQL.

Implement `lock.rs` with one `locks/maintenance.lock`: `HistoryStore::activity()` acquires a bounded shared guard and `HistoryStore::maintenance()` acquires a bounded exclusive guard. Use Rust 1.97's `File::try_lock_shared`, `File::try_lock`, and `File::unlock` in an interruptible deadline loop; do not add a locking crate. Guards are RAII, process teardown releases them, acquisition precedes database transactions, and no upgrade API exists. Pass guards into internal operations so they never reacquire or upgrade the same lock.

Run: `cargo test -p compass-history --test sqlite_contract && cargo clippy -p compass-history --all-targets -- -D warnings`

Expected: PASS; Cargo resolves exactly `prolly-map 0.5.0` and `prolly-store-sqlite 0.3.0`.

- [ ] **Step 6: Commit the crate boundary**

```bash
git add Cargo.toml Cargo.lock crates/compass-history
git commit -m "feat(history): add prolly-backed history crate"
```

### Task 2: Add canonical values, typed keys, and extraction fingerprints

**Files:**
- Create: `compass/crates/compass-history/src/canonical.rs`
- Create: `compass/crates/compass-history/src/keys.rs`
- Create: `compass/crates/compass-history/src/fingerprint.rs`
- Modify: `compass/crates/compass-history/src/lib.rs`
- Create: `compass/crates/compass-history/tests/canonical.rs`

**Interfaces:**
- Consumes: arbitrary `serde_json::Value`, Compass graph IDs, edge direction, and explicit extraction inputs.
- Produces: `canonical_json_bytes`, `node_key`, `edge_key`, `hyperedge_key`, `BuildProfile::digest`, and `ExtractionFingerprintInput::digest`.

- [ ] **Step 1: Write failing canonicalization and key tests**

```rust
use serde_json::json;
use compass_history::{
    ExtractionFingerprintInput, canonical_json_bytes, edge_key, hyperedge_key, node_key,
};

#[test]
fn object_order_and_separator_bytes_do_not_change_identity() {
    let left = json!({"z": 1, "a": {"y": 2, "x": 3}});
    let right = json!({"a": {"x": 3, "y": 2}, "z": 1});
    assert_eq!(canonical_json_bytes(&left).unwrap(), canonical_json_bytes(&right).unwrap());
    assert_ne!(node_key("a\0b"), node_key("a"));
    assert_ne!(edge_key("a", "b\0c", "calls", true, None), edge_key("a\0b", "c", "calls", true, None));
    let record = canonical_json_bytes(&json!({"members":["a","b"]})).unwrap();
    assert_ne!(hyperedge_key(&record, Some(0)), hyperedge_key(&record, Some(1)));
}

#[test]
fn fingerprint_is_order_independent_and_secret_free() {
    let mut input = ExtractionFingerprintInput::new("0.1.0", "schema-1");
    input.insert("model", "claude-sonnet").unwrap();
    input.insert("prompt_sha256", "abc123").unwrap();
    let digest = input.digest().unwrap();
    assert_eq!(digest.as_hex().len(), 64);
    assert!(input.insert("api_key", "must-not-enter-fingerprint").is_err());
    assert!(!String::from_utf8(input.canonical_bytes().unwrap()).unwrap().contains("api_key"));
}
```

- [ ] **Step 2: Run the tests to verify missing APIs**

Run: `cargo test -p compass-history --test canonical`

Expected: FAIL with unresolved imports from `compass_history`.

- [ ] **Step 3: Implement recursive canonical JSON**

```rust
// src/canonical.rs
pub const CANONICAL_ENCODING_VERSION: u32 = 1;

pub fn canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>, HistoryError> {
    let mut output = Vec::new();
    write_canonical_value(value, &mut output)?;
    Ok(output)
}
```

`write_canonical_value` recursively sorts object keys by UTF-8 bytes, preserves arrays,
emits integers as minimal decimal, and emits finite floats with deterministic shortest
round-trip formatting while preserving float identity (`1.0` is not integer `1` and
negative zero remains negative zero). It rejects non-finite numbers. Add golden byte
fixtures for integer boundaries, exponent forms, `1` versus `1.0`, negative zero, Unicode
escaping, and recursive ordering. The store-format record includes
`CANONICAL_ENCODING_VERSION`; changing a golden byte requires migration.

Add error variants:

```rust
#[error("canonical encoding failed: {0}")]
Canonical(String),
#[error("invalid typed key: {0}")]
InvalidKey(String),
```

- [ ] **Step 4: Implement segment-safe keys using Prolly's key contract**

```rust
// src/keys.rs
use prolly::KeyBuilder;

const KEY_SCHEMA_V1: &[u8] = &[1];
const NODE_KIND: &[u8] = &[1];
const EDGE_KIND: &[u8] = &[2];
const HYPEREDGE_KIND: &[u8] = &[3];

pub fn node_key(id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push_segment(KEY_SCHEMA_V1)
        .push_segment(NODE_KIND)
        .push_str(id)
        .finish()
}

pub fn edge_key(
    source: &str,
    target: &str,
    relation: &str,
    directed: bool,
    discriminator: Option<&[u8]>,
) -> Vec<u8> {
    let (source, target) = if directed || source <= target {
        (source, target)
    } else {
        (target, source)
    };
    let builder = KeyBuilder::new()
        .push_segment(KEY_SCHEMA_V1)
        .push_segment(EDGE_KIND)
        .push_str(source)
        .push_str(target)
        .push_str(relation);
    match discriminator {
        Some(value) => builder.push_segment(value).finish(),
        None => builder.finish(),
    }
}

pub fn hyperedge_key(identity: &[u8], occurrence: Option<u64>) -> Vec<u8> {
    let builder = KeyBuilder::new()
        .push_segment(KEY_SCHEMA_V1)
        .push_segment(HYPEREDGE_KIND)
        .push_segment(identity);
    match occurrence {
        Some(rank) => builder.push_segment(&rank.to_be_bytes()).finish(),
        None => builder.finish(),
    }
}

pub(crate) fn root_name(parts: &[&[u8]]) -> Vec<u8> {
    parts
        .iter()
        .fold(KeyBuilder::new(), |builder, part| builder.push_segment(part))
        .finish()
}
```

- [ ] **Step 5: Implement explicit fingerprint inputs**

Define `BuildProfile` separately from `ExtractionFingerprintInput`. Queue deduplication uses
the normalized non-secret profile digest. Only after the exact target worktree exists does
the worker add target-commit configuration, every applied committed `.gitignore` and
`.graphifyignore`, explicit excludes, prompt/schema/extractor/resolver/pipeline versions,
features, direction, multigraph behavior, and clustering settings to the final fingerprint.
Never include `.git/info/exclude`, global ignores, credentials, absolute paths, time,
concurrency, timeouts, or diagnostics.

```rust
// src/fingerprint.rs
use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{HistoryError, canonical_json_bytes};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractionFingerprint([u8; 32]);

impl ExtractionFingerprint {
    #[must_use]
    pub fn as_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct BuildProfile {
    values: BTreeMap<String, String>,
}

impl BuildProfile {
    pub fn digest(&self) -> Result<[u8; 32], HistoryError> {
        let bytes = canonical_json_bytes(&serde_json::to_value(&self.values)
            .map_err(|error| HistoryError::Canonical(error.to_string()))?)?;
        Ok(Sha256::digest(bytes).into())
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExtractionFingerprintInput {
    values: BTreeMap<String, String>,
}

impl ExtractionFingerprintInput {
    pub fn new(compass_version: &str, graph_schema: &str) -> Self {
        let mut input = Self::default();
        input.values.insert("compass_version".to_owned(), compass_version.to_owned());
        input.values.insert("graph_schema".to_owned(), graph_schema.to_owned());
        input
    }

    pub fn insert(&mut self, key: &str, value: &str) -> Result<(), HistoryError> {
        let normalized = key.to_ascii_lowercase();
        if ["key", "secret", "token", "password", "credential"]
            .iter()
            .any(|needle| normalized.contains(needle))
        {
            return Err(HistoryError::FingerprintSecretKey(key.to_owned()));
        }
        self.values.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, HistoryError> {
        canonical_json_bytes(&serde_json::to_value(&self.values)
            .map_err(|error| HistoryError::Canonical(error.to_string()))?)
    }

    pub fn digest(&self) -> Result<ExtractionFingerprint, HistoryError> {
        Ok(ExtractionFingerprint(Sha256::digest(self.canonical_bytes()?).into()))
    }
}
```

Add `HistoryError::FingerprintSecretKey(String)`. Derive Serde traits for
`ExtractionFingerprint`, implement strict 64-character lowercase-hex parsing, and use
the same parser when reading manifests and jobs.

- [ ] **Step 6: Export the APIs and run focused tests**

Run: `cargo test -p compass-history --test canonical && cargo clippy -p compass-history --all-targets -- -D warnings`

Expected: PASS, including object-order and separator-byte cases.

- [ ] **Step 7: Commit canonical identity**

```bash
git add crates/compass-history
git commit -m "feat(history): define canonical graph identities"
```

### Task 3: Define version manifests and lossless artifact partitioning

**Files:**
- Create: `compass/crates/compass-history/src/model.rs`
- Create: `compass/crates/compass-history/src/artifacts.rs`
- Modify: `compass/crates/compass-history/src/lib.rs`
- Create: `compass/crates/compass-history/tests/roundtrip.rs`

**Interfaces:**
- Consumes: `GraphDocument`, authoritative analysis/labels/manifest/opaque sidecars, `CompletionEvidence`, and Prolly `Tree` handles.
- Produces: `GraphArtifacts::{load, partition, reconstruct, write_seed}`, `ArtifactRegistryEntry`, `GraphVersion`, `StoredTree`, and `RealizationId`.

- [ ] **Step 1: Write a failing graph artifact round-trip test**

```rust
use serde_json::json;
use compass_history::{CompletionEvidence, GraphArtifacts};
use compass_model::GraphDocument;

#[test]
fn complete_graph_and_build_state_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": false,
        "multigraph": true,
        "graph": {"hyperedges": [{"id":"flow","nodes":["a","b"]}]},
        "nodes": [
            {"id":"a","label":"A","community":1,"_origin":"ast"},
            {"id":"b","label":"B","community":1,"_origin":"semantic"}
        ],
        "links": [
            {"source":"a","target":"b","relation":"calls","confidence":"INFERRED"},
            {"source":"a","target":"b","relation":"calls","confidence":"INFERRED"}
        ],
        "hyperedges": [
            {"nodes":["a","b"]},
            {"nodes":["a","b"]}
        ],
        "built_at_commit": "0123456789abcdef"
    }))?;
    let artifacts = GraphArtifacts {
        document: document.clone(),
        analysis: Some(json!({"communities":{"1":["a","b"]}})),
        labels: Some(json!({"1":"Core"})),
        manifest: Some(json!({"a.py":{"ast_hash":"abc","semantic_hash":"abc","mtime":1.0}})),
        authoritative_sidecars: Default::default(),
    };
    let completion = CompletionEvidence {
        extraction_succeeded: true,
        allow_partial: false,
        semantic_files_expected: 1,
        semantic_files_completed: 1,
        failed_chunks: 0,
    };
    let partitioned = artifacts.partition(&completion)?;
    let restored = GraphArtifacts::reconstruct(&partitioned)?;
    assert_eq!(restored.document, document);
    assert_eq!(restored.analysis, artifacts.analysis);
    assert_eq!(restored.labels, artifacts.labels);
    assert_eq!(restored.manifest, artifacts.manifest);
    Ok(())
}
```

- [ ] **Step 2: Run the round-trip test and verify missing types**

Run: `cargo test -p compass-history --test roundtrip complete_graph_and_build_state_round_trip`

Expected: FAIL with unresolved `GraphArtifacts`.

- [ ] **Step 3: Define content-only tree and realization types**

```rust
// src/model.rs
use prolly::{Cid, Config, RuntimeConfig, Tree, TreeFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{HistoryError, canonical_json_bytes};

pub const HISTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactClass { Authoritative, Derived, Operational }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRegistryEntry {
    pub registry_version: u32,
    pub relative_path: String,
    pub class: ArtifactClass,
    pub media_type: String,
    pub schema_version: Option<u32>,
    pub content_digest: Option<[u8; 32]>,
    pub storage: Option<Vec<u8>>,
    pub regeneration_version: Option<String>,
}

pub type ArtifactContent = Vec<u8>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredTree {
    pub root: Option<[u8; 32]>,
    pub format: TreeFormat,
}

impl StoredTree {
    pub fn from_tree(tree: &Tree) -> Self {
        Self {
            root: tree.root.as_ref().map(|cid| cid.0),
            format: tree.config.format.clone(),
        }
    }

    pub fn to_tree(&self) -> Tree {
        Tree {
            root: self.root.map(Cid),
            config: Config {
                format: self.format.clone(),
                runtime: RuntimeConfig::default(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GraphVersion {
    pub schema_version: u32,
    pub git_commit: String,
    pub git_parents: Vec<String>,
    pub extraction_fingerprint: String,
    pub nodes_root: StoredTree,
    pub edges_root: StoredTree,
    pub hyperedges_root: StoredTree,
    pub analysis_root: StoredTree,
    pub metadata_root: StoredTree,
    pub node_count: u64,
    pub edge_count: u64,
    pub hyperedge_count: u64,
    pub analysis_count: u64,
    pub metadata_count: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RealizationId([u8; 32]);

impl RealizationId {
    pub fn for_version(version: &GraphVersion) -> Result<Self, HistoryError> {
        let value = serde_json::to_value(version)
            .map_err(|error| HistoryError::Canonical(error.to_string()))?;
        Ok(Self(Sha256::digest(canonical_json_bytes(&value)?).into()))
    }

    pub fn as_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
```

- [ ] **Step 4: Implement artifact loading and typed partition maps**

`GraphArtifacts::load` reads `graph.json`, `.graphify_analysis.json`, `.graphify_labels.json`, `manifest.json`, and every artifact declared authoritative by the versioned registry. It accepts `CompletionEvidence` only through `CompletedGraphArtifacts`; loading files alone is not proof that semantic extraction completed. Raw `.graphify_semantic_marker`, `cost.json`, timings, diagnostics, caches, `.graphify_root`, and learning overlays are operational and remain outside identity. `partition` returns:

```rust
pub struct PartitionedGraph {
    pub nodes: Vec<(Vec<u8>, Vec<u8>)>,
    pub edges: Vec<(Vec<u8>, Vec<u8>)>,
    pub hyperedges: Vec<(Vec<u8>, Vec<u8>)>,
    pub analysis: Vec<(Vec<u8>, Vec<u8>)>,
    pub metadata: Vec<(Vec<u8>, Vec<u8>)>,
}
```

For node values, move only `community`, `community_name`, and `norm_label` into an analysis record keyed by node ID. Simple edge keys use direction-aware endpoints and relation. Multigraph keys append the canonical NetworkX `key` when present, otherwise the complete canonical-edge digest plus its zero-based occurrence among byte-identical records. For hyperedges, reject duplicate explicit IDs; id-less records use the complete canonical-record digest plus their zero-based occurrence among byte-identical records.

Metadata records node, edge, and hyperedge input order with fixed-width big-endian rank keys pointing to complete typed record keys, whether the document used `links` or legacy `edges`, original hyperedge placement, unknown graph/top-level values, completion evidence, normalized semantic provenance, graph/export versions, and the artifact registry. Registry entries contain relative path, class (`authoritative`, `derived`, or `operational`), media/schema type, content digest, storage, and regeneration version. Store non-derivable authoritative sidecars verbatim. Mark `GRAPH_REPORT.md`, HTML, visualization output, and label signatures as derived. Unknown files declared authoritative by the current artifact contract may not be silently dropped.

Use one versioned raw envelope for every value:

```rust
fn encode_record(schema: &str, value: &serde_json::Value) -> Result<Vec<u8>, HistoryError> {
    let payload = crate::canonical_json_bytes(value)?;
    prolly::VersionedValue::raw(schema, 1, payload)
        .to_bytes()
        .map_err(HistoryError::from)
}
```

- [ ] **Step 5: Implement reconstruction and seed export**

`GraphArtifacts::reconstruct` decodes all five record groups, rejoins the three derived node attributes, restores order, legacy spelling, multigraph discriminators, hyperedge placement, unknown values, and authoritative sidecars, then rebuilds `GraphDocument`. `write_seed(output_dir)` writes only compatible authoritative inputs: `graph.json`, analysis, labels, manifest, opaque authoritative sidecars, and a semantic marker generated from completion evidence using `compass_files::{write_bytes_atomic, write_json_atomic}`. It never renders derived files. Task 7 owns report/HTML regeneration through `compass-output` and the recorded renderer version; byte equality is not required, but canonical semantic inputs must match.

The implementation must reject duplicate node keys, duplicate non-multigraph identities,
duplicate explicit hyperedge IDs, malformed envelopes, invalid occurrence discriminators,
and analysis/order records that reference missing typed keys. It must preserve parallel
multigraph edges and exact duplicate id-less hyperedges.

- [ ] **Step 6: Add path, Unicode, unknown-field, and no-sidecar cases**

Extend `roundtrip.rs` with table-driven cases containing NUL bytes in IDs, non-ASCII labels, legacy `edges`, unknown top-level fields, directed/undirected simple and multigraph inputs, exact duplicate edges and id-less hyperedges, absent hyperedges, artifact classes, and absent sidecars. Assert exact `GraphDocument` and authoritative-sidecar equality after reconstruction; assert that changing only raw token/cost/timing provenance does not change partitioned identity.

- [ ] **Step 7: Run crate tests and lint**

Run: `cargo test -p compass-history --test roundtrip && cargo clippy -p compass-history --all-targets -- -D warnings`

Expected: PASS with no lossy-field exceptions.

- [ ] **Step 8: Commit the graph persistence schema**

```bash
git add crates/compass-history
git commit -m "feat(history): partition complete graph artifacts"
```

### Task 4: Build and atomically publish typed Prolly roots

**Files:**
- Modify: `compass/crates/compass-history/src/store.rs`
- Modify: `compass/crates/compass-history/src/model.rs`
- Modify: `compass/crates/compass-history/src/lib.rs`
- Modify: `compass/crates/compass-history/tests/publication.rs`

**Interfaces:**
- Consumes: `PublishRequest` and `PartitionedGraph`.
- Produces: `HistoryStore::{publish, preferred, compare_and_set_preferred, get, list}`, crate-private `publish_with_activity`, and immutable named roots for all realization components. The exact lookup signature is `preferred(&self, commit: &CommitId)`; the setter takes the commit, expected realization (or `None`), and new realization.

- [ ] **Step 1: Write failing publication, reopen, and conflict tests**

```rust
#[test]
fn publication_is_atomic_reopenable_and_content_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = committed_repository()?;
    let repository = Repository::discover(fixture.path())?;
    let history = HistoryStore::create(&repository)?;
    let request = fixture_publish_request("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let first = history.publish(request.clone())?;
    let second = history.publish(request)?;
    assert_eq!(first.id, second.id);
    drop(history);

    let reopened = HistoryStore::open_existing(&repository)?.expect("history store");
    let commit = first.version.git_commit.parse()?;
    assert_eq!(reopened.preferred(&commit)?.unwrap().id, first.id);
    assert_eq!(reopened.get(&first.id)?.version, first.version);
    assert_eq!(reopened.list(None)?.len(), 1);
    Ok(())
}
```

Add a second test that publishes two different fingerprints for one commit, asserts both remain listed, and asserts only the requested result becomes preferred.

- [ ] **Step 2: Run the publication tests to verify missing APIs**

Run: `cargo test -p compass-history --test publication publication_is_atomic_reopenable_and_content_idempotent`

Expected: FAIL because `PublishRequest` and publication methods do not exist.

- [ ] **Step 3: Add stable commit and publication request types**

```rust
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommitId(String);

impl std::str::FromStr for CommitId {
    type Err = HistoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let valid_len = matches!(value.len(), 40 | 64);
        if valid_len && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(HistoryError::InvalidCommit(value.to_owned()))
        }
    }
}

impl CommitId {
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl std::fmt::Display for CommitId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone)]
pub struct PublishRequest {
    pub commit: CommitId,
    pub parents: Vec<CommitId>,
    pub fingerprint: ExtractionFingerprint,
    pub artifacts: GraphArtifacts,
    pub completion: CompletionEvidence,
    pub make_preferred: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PublishedVersion {
    pub id: RealizationId,
    pub version: GraphVersion,
    pub preferred: bool,
}
```

- [ ] **Step 4: Build each tree with deterministic `BatchBuilder`**

```rust
fn build_tree(
    store: std::sync::Arc<prolly_store_sqlite::SqliteStore>,
    entries: Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<prolly::Tree, HistoryError> {
    let mut builder = prolly::BatchBuilder::new(store, prolly::Config::default());
    for (key, value) in entries {
        builder.add(key, value);
    }
    builder.build().map_err(HistoryError::from)
}
```

Expose the backend clone from `HistoryStore` only inside the crate. Public `publish` acquires a shared activity guard; crate-private `publish_with_activity` accepts an existing guard so materialization never nests file locks. Hold the guard across building, validation, catalog publication, reopen verification, and preferred selection so exclusive GC cannot delete unpublished nodes. Build nodes, edges, hyperedges, analysis, and metadata trees; derive `GraphVersion`; then build a one-entry manifest tree containing canonical manifest bytes under `KeyBuilder::new().push_str("manifest").finish()`.

- [ ] **Step 5: Publish the realization and preferred pointer with separate CAS boundaries**

Use named roots with segment-safe names:

```text
compass/v1/version/<realization-id>/nodes
compass/v1/version/<realization-id>/edges
compass/v1/version/<realization-id>/hyperedges
compass/v1/version/<realization-id>/analysis
compass/v1/version/<realization-id>/metadata
compass/v1/version/<realization-id>/manifest
compass/v1/preferred/<commit>
```

First publish all six immutable realization roots in one strict transaction. Treat a
pre-existing identical realization root as idempotent; reject a same-name/different-root
collision as corruption. This transaction is the version-catalog CAS and makes the
realization fully addressable before it can be preferred.

Reopen the realization through its six catalog roots and verify its digest and direct roots
before preferred selection. When `make_preferred` is true, validate the preferred realization
observed before publication. A corrupt preferred leaves the new realization addressable but
non-preferred and returns a corruption diagnostic. Otherwise compare-and-swap
`compass/v1/preferred/<commit>` from the
value observed at the start of publication to the new manifest tree. A stale preferred
CAS is not rolled back and does not overwrite the concurrent winner: return the
published realization with `preferred: false`. Otherwise return `preferred: true`.
This preserves both distinct concurrent results while readers see only complete roots.
Expose the same rule through `compare_and_set_preferred` for the explicit CLI command.
Replacing corrupt preferred state is a separate recovery API requiring its exact observed ID,
a newly validated candidate, and `--replace-corrupt`; concurrent repair returns conflict.

- [ ] **Step 6: Implement preferred lookup, exact lookup, and listing**

Use `ManifestStoreScan::list_roots`, `prolly::decode_segments`, and the manifest tree's single record. `list(Some(commit))` filters by the exact commit recorded in each manifest rather than trusting a filename alone. Sort results by commit, fingerprint, and realization ID for deterministic CLI output.

- [ ] **Step 7: Run persistence and concurrency tests**

Run: `cargo test -p compass-history --test publication && cargo clippy -p compass-history --all-targets -- -D warnings`

Expected: PASS for reopen, idempotency, multiple realizations, stale preferred CAS, and transaction rollback.

- [ ] **Step 8: Commit atomic publication**

```bash
git add crates/compass-history
git commit -m "feat(history): publish immutable graph versions"
```

### Task 5: Validate published realizations and reject corrupt graphs

**Files:**
- Create: `compass/crates/compass-history/src/validate.rs`
- Modify: `compass/crates/compass-history/src/store.rs`
- Modify: `compass/crates/compass-history/src/error.rs`
- Modify: `compass/crates/compass-history/src/lib.rs`
- Modify: `compass/crates/compass-history/tests/publication.rs`

**Interfaces:**
- Consumes: `PublishedVersion`, its five trees, and decoded records.
- Produces: `HistoryStore::validate(&RealizationId) -> Result<ValidationReport, HistoryError>` and pre-publication validation inside `publish`.

- [ ] **Step 1: Write failing validation tests**

```rust
#[test]
fn validation_rejects_missing_endpoints_and_count_mismatches() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = committed_repository()?;
    let repository = Repository::discover(fixture.path())?;
    let history = HistoryStore::create(&repository)?;
    let mut request = fixture_publish_request("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    request.artifacts.document.links[0].target = "missing".to_owned();
    let error = history.publish(request).unwrap_err();
    assert!(error.to_string().contains("missing edge endpoint"));
    assert!(history.list(None)?.is_empty());
    Ok(())
}
```

Add tests using a deliberately corrupt prebuilt SQLite fixture or a test-only store wrapper to alter manifest bytes, hide a referenced CID, inject an invalid multigraph discriminator, break an order reference, and inject a missing hyperedge member. Do not mutate adapter-private SQL tables. Each case must return a typed validation error and must not silently select another version.

- [ ] **Step 2: Run the focused test and confirm publication currently accepts the bad edge**

Run: `cargo test -p compass-history --test publication validation_rejects_missing_endpoints_and_count_mismatches`

Expected: FAIL because `publish` does not yet reject the missing endpoint.

- [ ] **Step 3: Add structured validation output**

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidationReport {
    pub nodes: u64,
    pub edges: u64,
    pub hyperedges: u64,
    pub analysis_records: u64,
    pub metadata_records: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationProblem {
    RealizationDigest,
    MissingRoot(&'static str),
    Count { kind: &'static str, expected: u64, actual: u64 },
    KeyMismatch { kind: &'static str, key: Vec<u8> },
    MissingEdgeEndpoint { edge: Vec<u8>, endpoint: String },
    MissingHyperedgeMember { hyperedge: Vec<u8>, member: String },
    MissingAnalysisNode(String),
    MissingOrderRecord { kind: &'static str, key: Vec<u8> },
    InvalidMultigraphDiscriminator(Vec<u8>),
    DuplicateExplicitHyperedgeId(String),
    ArtifactRegistry(String),
    IncompleteSemanticState,
}
```

Add `HistoryError::InvalidRealization(Vec<ValidationProblem>)` with a concise display summary.

Define these versioned v1 limits in one public policy type rather than scattering literals:

```rust
pub const MAX_AUTHORITATIVE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_KEY_BYTES: usize = 1024 * 1024;
pub const MAX_RECORD_VALUE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_RECORDS_PER_TREE: u64 = 10_000_000;
pub const MAX_JSON_DEPTH: usize = 128;
pub const MAX_JOB_BYTES: usize = 1024 * 1024;
pub const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
```

Check lengths before allocation and check nesting while decoding. Tests cross every boundary
with compact counting readers/builders so they do not allocate hundreds of MiB.

- [ ] **Step 4: Implement streaming validation**

Scan each tree with counters and a bounded decode buffer. Verify endpoint/member/reference
existence with point lookups into the immutable node/record trees, optionally through a fixed-
capacity cache; never retain all node IDs or records in memory. Verify:

- recomputed manifest digest equals `RealizationId`;
- every `StoredTree` root opens;
- all five record counts, including analysis and metadata, equal the manifest;
- decoded record identity equals its typed key;
- multigraph and id-less-hyperedge discriminators are valid and duplicate-safe;
- every edge endpoint and hyperedge member exists;
- node-scoped analysis references exist;
- order records resolve to existing typed keys and the artifact registry is complete;
- canonical envelopes round-trip within the exact v1 key/value/count/depth limits above;
- completion evidence says the pipeline succeeded, `allow_partial` was false,
  expected and completed semantic-file counts match, and failed chunks are zero.

Return every discovered problem in one error so `compass history status` can report a complete diagnosis.

- [ ] **Step 5: Validate before the publication transaction**

Build all trees and the candidate manifest, validate them by their direct immutable roots, and only then enter the named-root transaction. After commit, reopen the realization and run the lightweight digest/root check once more. A failed post-commit check returns corruption without changing preferred to another realization.

- [ ] **Step 6: Run validation and all history tests**

Run: `cargo test -p compass-history && cargo clippy -p compass-history --all-targets -- -D warnings`

Expected: PASS; invalid graphs leave the version catalog unchanged.

- [ ] **Step 7: Commit fail-closed validation**

```bash
git add crates/compass-history
git commit -m "feat(history): validate graph realizations before publish"
```

### Task 6: Stream graph-aware diffs across typed roots

**Files:**
- Create: `compass/crates/compass-history/src/diff.rs`
- Modify: `compass/crates/compass-history/src/lib.rs`
- Create: `compass/crates/compass-history/tests/diff.rs`

**Interfaces:**
- Consumes: two `RealizationId` values and Prolly `Diff` entries.
- Produces: `ChangeSink`, `HistoryStore::diff(old, new, sink)` and stable `GraphChange` records.

- [ ] **Step 1: Write a failing typed diff test**

```rust
use compass_history::{ChangeKind, ChangeSink, GraphChange, HistoryError, RecordKind};

#[derive(Default)]
struct VecChangeSink(Vec<GraphChange>);

impl ChangeSink for VecChangeSink {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        self.0.push(change);
        Ok(())
    }
}

#[test]
fn diff_reports_topology_and_attribute_changes() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let history = fixture_history(directory.path())?;
    let old = history.publish(version_one())?;
    let new = history.publish(version_two())?;
    let mut changes = VecChangeSink::default();
    history.diff(&old.id, &new.id, &mut changes)?;
    assert!(changes.0.iter().any(|change| change.record == RecordKind::Node && change.change == ChangeKind::Added));
    assert!(changes.0.iter().any(|change| change.record == RecordKind::Edge && change.change == ChangeKind::Changed));
    assert!(changes.0.iter().any(|change| change.record == RecordKind::Analysis));
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify the diff API is missing**

Run: `cargo test -p compass-history --test diff diff_reports_topology_and_attribute_changes`

Expected: FAIL with unresolved diff types and method.

- [ ] **Step 3: Define stable diff records**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind { Node, Edge, Hyperedge, Analysis, Metadata }

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind { Added, Removed, Changed }

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct GraphChange {
    pub record: RecordKind,
    pub change: ChangeKind,
    pub key: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<serde_json::Value>,
}

pub trait ChangeSink {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError>;
}
```

- [ ] **Step 4: Adapt `prolly::Diff` without buffering both graphs**

For each tree pair, return immediately when `StoredTree` values are equal. Otherwise call `Prolly::stream_diff`, decode `KeyBuilder` segments with `decode_segments`, decode versioned record envelopes, and invoke the sink once per change:

```rust
pub fn diff(
    &self,
    old: &RealizationId,
    new: &RealizationId,
    sink: &mut dyn ChangeSink,
) -> Result<(), HistoryError> {
    let _activity = self.activity()?;
    let old = self.get(old)?;
    let new = self.get(new)?;
    self.diff_root(RecordKind::Node, &old.version.nodes_root, &new.version.nodes_root, sink)?;
    self.diff_root(RecordKind::Edge, &old.version.edges_root, &new.version.edges_root, sink)?;
    self.diff_root(RecordKind::Hyperedge, &old.version.hyperedges_root, &new.version.hyperedges_root, sink)?;
    self.diff_root(RecordKind::Analysis, &old.version.analysis_root, &new.version.analysis_root, sink)?;
    self.diff_root(RecordKind::Metadata, &old.version.metadata_root, &new.version.metadata_root, sink)
}
```

- [ ] **Step 5: Add relation-change and synthetic-hyperedge expectations**

Test that changing an edge relation yields one removed edge and one added edge. Test that a hyperedge with explicit `id` yields `Changed`, while a synthetic digest key yields removal plus addition. Add exact duplicate multigraph-edge and id-less-hyperedge occurrence tests, and assert a failing sink stops traversal without buffering remaining changes.

- [ ] **Step 6: Prove equal roots perform no record reads**

Wrap the store in a test-only counting adapter, diff a realization against itself, and assert the sink is empty and node-read metrics do not increase. Do not depend on dependency-internal `#[cfg(test)]` APIs, which are unavailable to downstream crates.

- [ ] **Step 7: Run tests and commit**

Run: `cargo test -p compass-history --test diff && cargo clippy -p compass-history --all-targets -- -D warnings`

```bash
git add crates/compass-history
git commit -m "feat(history): stream graph-aware version diffs"
```

### Task 7: Expose explicit history inspection and export commands

**Files:**
- Modify: `compass/crates/compass-history/src/git.rs`
- Create: `compass/crates/compass-history/tests/git.rs`
- Modify: `compass/crates/compass-cli/Cargo.toml`
- Create: `compass/crates/compass-cli/src/history_commands.rs`
- Modify: `compass/crates/compass-cli/src/lib.rs`
- Create: `compass/crates/compass-cli/tests/history_cli.rs`
- Create: `compass/crates/compass-output/src/history_bundle.rs`
- Modify: `compass/crates/compass-output/src/lib.rs`
- Create: `compass/crates/compass-output/tests/history_bundle.rs`

**Interfaces:**
- Consumes: `HistoryStore::{open_existing, preferred, get, list, validate}`, `GraphArtifacts::write_seed`, the artifact registry, and versioned `compass-output` renderers.
- Produces: `Repository::{resolve, parents}` on the existing discovery type, plus canonical `compass history status`, `compass history list`, `compass history show`, `compass history prefer`, and `compass history export` CLI commands.

- [ ] **Step 1: Write failing CLI help and empty-store tests**

```rust
#[test]
fn history_help_and_empty_status_are_actionable() -> Result<(), Box<dyn std::error::Error>> {
    let repository = initialized_repository()?;
    let help = run_compass(repository.path(), &["history", "--help"])?;
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("history build <rev>"));

    let status = run_compass(repository.path(), &["history", "status", "HEAD"])?;
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("disabled"));
    assert!(!repository.path().join(".git/compass").exists());

    Ok(())
}
```

- [ ] **Step 2: Run the test and verify `compass history` is unknown**

Run: `cargo test -p compass-cli --test history_cli history_help_and_empty_status_are_actionable`

Expected: FAIL because command dispatch reports an unknown command.

- [ ] **Step 3: Add CLI dependency and command dispatch**

Add `compass-history` to `compass-cli/Cargo.toml`, declare `mod history_commands;`, and dispatch:

```rust
"history" => history_commands::command_history(frontend, &args),
"diff" => history_commands::command_diff(frontend, &args),
```

Add both commands to `compass_help` and `compass_command_help`.
`history_help(frontend)` must show the complete approved command family even before `build` is
wired in Task 11. Refactor query/path/explain usage generation so Compass diagnostics always
render the canonical executable name.

- [ ] **Step 4: Implement repository and revision discovery once**

Add `Repository { root: PathBuf, common_dir: PathBuf }` in `compass/crates/compass-history/src/git.rs`. `Repository::discover(current_dir)` runs:

```text
git rev-parse --show-toplevel
git rev-parse --git-common-dir
```

Resolve relative common-dir output against the repository root and canonicalize it. `resolve(revision)` runs `git rev-parse --verify --end-of-options <revision>^{commit}`, parses a 40- or 64-hex `CommitId`, and rejects stderr/stdout with additional lines. `parents(commit)` runs `git show -s --format=%P <commit>` and parses every parent.

Add tests for a normal repository, a linked worktree, an unknown revision, SHA-1 and SHA-256 repositories, and a revision beginning with `-`. `history_commands::open_existing_repository_history` calls `HistoryStore::open_existing(&repository)` and treats `None` as a normal no-store state; mutating commands introduced later call `HistoryStore::create`. Both target `<repository.common_dir()>/compass/history.sqlite`; no CLI code reimplements Git path or SQLite rules.

- [ ] **Step 5: Implement read-only command output**

- `status [rev] [--format text|json]`: enablement, store presence/compatibility, preferred realization ID, fingerprint, counts, and validation result. An absent store reports `disabled` and `no store`, exits `0`, and creates nothing.
- `list [rev] [--format text|json]`: deterministic tabular lines or a JSON array for every realization; absent and empty stores are successful empty results.
- `show <id> [--format text|json]`: text summary or JSON `GraphVersion`.
- `prefer <rev> <id>`: verify the manifest's commit matches the revision, read the current
  preferred realization, validate it, then call `compare_and_set_preferred`; report a concurrent
  change instead of overwriting it. If the current preferred is corrupt, fail and direct the
  user to `compass history rebuild <rev> --replace-corrupt` rather than bypassing recovery controls.
- `export <rev> --format graph-json --output <path>`: reconstruct and atomically write canonical `graph.json`; reject directories for this mode.
- `export <rev> --format graphify-out --output <directory>`: stage a complete bundle beside the destination, write stored authoritative sidecars verbatim, regenerate registered derived reports/HTML using the recorded renderer versions, validate the bundle, then atomically publish it. Fail closed when a required renderer version is unavailable; never silently substitute the current renderer.

Bundle output must not already exist; v1 has no implicit merge or destructive `--force` mode.
Graph-JSON output atomically replaces only the exact file explicitly named by the user.

In `compass-output/src/history_bundle.rs`, expose a renderer table keyed by the exact
`regeneration_version` values emitted by Task 3. The API accepts ordinary graph/sidecar values,
not `compass-history` types, so dependencies remain acyclic. Each supported version renders into
a staging directory; an unknown version is a typed error. Tests pin canonical-semantic output
for every registered version and prove the current renderer is never substituted implicitly.

Implement the global exit/output contract: `0` for valid empty/no-store reads, `2` for argument
errors, and `1` for repository, output, storage, or validation failures. Text/JSON results use
stdout; diagnostics use stderr. Status renders its report but exits `1` for an incompatible
store or corrupt selected preferred realization. Test short and broken output writers.

- [ ] **Step 6: Seed CLI tests through the library**

In `history_cli.rs`, publish fixtures directly through `HistoryStore::create` under the test repository's common directory, then assert `list`, `show`, `prefer`, graph-JSON export, and bundle export through `compass`. This keeps CLI presentation tests independent from historical build orchestration and from any legacy executable.

- [ ] **Step 7: Run CLI and coverage-boundary tests**

Run: `cargo test -p compass-cli --test history_cli && cargo test -p compass-cli --test coverage_paths`

Expected: PASS; add invalid `compass history` and `compass diff` argument cases to `coverage_paths.rs`.

- [ ] **Step 8: Commit explicit history inspection**

```bash
git add crates/compass-history/src/git.rs crates/compass-history/tests/git.rs crates/compass-cli crates/compass-output
git commit -m "feat(cli): inspect and export graph history"
```

### Task 8: Add CLI graph diff summaries, details, and JSON streaming

**Files:**
- Modify: `compass/crates/compass-cli/src/history_commands.rs`
- Modify: `compass/crates/compass-cli/tests/history_cli.rs`
- Modify: `compass/crates/compass-cli/tests/coverage_paths.rs`

**Interfaces:**
- Consumes: two resolved preferred versions and `GraphChange` callbacks.
- Produces: `compass diff A B`, `--detailed`, `--format json`, and `--topology-only` with the documented Compass process contract.

- [ ] **Step 1: Write failing summary and JSON tests**

```rust
#[test]
fn diff_supports_summary_and_machine_readable_output() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = history_fixture_with_two_commits()?;
    let summary = fixture.run_compass(&["diff", "HEAD~1", "HEAD"])?;
    assert!(summary.status.success());
    assert!(String::from_utf8_lossy(&summary.stdout).contains("1 node added"));

    let json = fixture.run_compass(&["diff", "HEAD~1", "HEAD", "--format", "json"])?;
    let changes: Vec<serde_json::Value> = serde_json::from_slice(&json.stdout)?;
    assert!(changes.iter().any(|change| change["record"] == "edge"));
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify `command_diff` is not implemented**

Run: `cargo test -p compass-cli --test history_cli diff_supports_summary_and_machine_readable_output`

Expected: FAIL with the temporary not-implemented history diff response.

- [ ] **Step 3: Parse revisions and mutually exclusive output flags**

Require exactly two revisions. Reject `--detailed` with `--format json`, reject unknown formats, and accept `--topology-only` with either output mode. Resolve both revisions to full commit IDs before store lookup.

- [ ] **Step 4: Implement bounded summary aggregation**

Count changes by `(RecordKind, ChangeKind)` while retaining at most 20 representative labels per category. Render stable singular/plural text. `--topology-only` filters out analysis and metadata before counting.

- [ ] **Step 5: Implement detailed and streaming JSON output**

Detailed mode renders one record at a time. JSON mode writes `[` then each serialized change separated by `,` and finally `]`; it must not collect all changes into a `Vec`. Add a private writer-based function so tests can inject a short-writing or failing sink.

Keep stdout machine-clean while either missing side lazily materializes: stage/progress messages
go only to stderr. A sink error stops the Prolly traversal and exits `1`.

- [ ] **Step 6: Test relation, confidence, analysis, and empty diffs**

Add fixtures for an edge relation replacement, confidence-only value change, community reassignment, and identical roots. Assert stable output ordering by record-kind order and Prolly key order.

- [ ] **Step 7: Run tests and commit**

Run: `cargo test -p compass-cli --test history_cli && cargo clippy -p compass-cli --all-targets -- -D warnings`

```bash
git add crates/compass-cli
git commit -m "feat(cli): diff graph history by commit"
```

### Task 9: Load already-materialized commits through `--at`

**Files:**
- Modify: `compass/crates/compass-core/src/lib.rs`
- Modify: `compass/crates/compass-cli/src/lib.rs`
- Modify: `compass/crates/compass-cli/src/history_commands.rs`
- Modify: `compass/crates/compass-cli/tests/history_cli.rs`
- Modify: `compass/crates/compass-cli/tests/coverage_paths.rs`

**Interfaces:**
- Consumes: a reconstructed `GraphArtifacts` for a preferred commit.
- Produces: `LoadedGraph::from_document`, `GraphSelection`, and `--at <rev>` for query, path, and explain.

- [ ] **Step 1: Write a failing historical query test**

```rust
#[test]
fn query_path_and_explain_read_the_selected_commit() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = history_fixture_with_two_commits()?;
    let query = fixture.run_compass(&["query", "legacy service", "--at", "HEAD~1"])?;
    assert!(query.status.success());
    assert!(String::from_utf8_lossy(&query.stdout).contains("LegacyService"));
    assert!(!String::from_utf8_lossy(&query.stdout).contains("ReplacementService"));

    let path = fixture.run_compass(&["path", "LegacyService", "Database", "--at=HEAD~1"])?;
    assert!(path.status.success());
    let explain = fixture.run_compass(&["explain", "LegacyService", "--at", "HEAD~1"])?;
    assert!(explain.status.success());
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify `--at` is ignored or rejected**

Run: `cargo test -p compass-cli --test history_cli query_path_and_explain_read_the_selected_commit`

Expected: FAIL because the current checkout's `graph.json` is loaded.

- [ ] **Step 3: Add an in-memory `LoadedGraph` constructor**

```rust
impl LoadedGraph {
    pub fn from_document(document: GraphDocument, force_directed: bool) -> Result<Self, GraphError> {
        let mut document = document;
        if force_directed {
            document.directed = true;
        }
        Ok(Self {
            graph: Graph::from_document(document)?,
            overlay: HashMap::new(),
        })
    }
}
```

Historical queries deliberately use an empty learning overlay because the current `.graphify_learning.json` is experiential state, not part of a commit realization.

- [ ] **Step 4: Parse one exclusive graph source**

```rust
enum GraphSelection {
    File(PathBuf),
    Commit(String),
}
```

Add a shared parser used by query, path, and explain. Accept `--at REV` and `--at=REV`. Reject `--at` combined with `--graph`, repeated selectors, missing values, or extra positionals. Preserve the existing default graph path when neither selector is present.

Change `command_query`, `command_path`, `command_explain`, and their help/usage helpers so the
Compass dispatcher, which has removed the mandatory `graph` segment, renders canonical
`compass ...` usage.

- [ ] **Step 5: Resolve and load materialized history**

For `GraphSelection::Commit`, acquire one shared activity guard spanning preferred lookup,
validation, reconstruction, and `LoadedGraph::from_document`; discover the repository,
resolve the revision, load its preferred realization, validate it, reconstruct
`GraphArtifacts`, and call `LoadedGraph::from_document`. If no preferred version exists, return:

```text
error: no graph realization for <commit>; run `compass history build <rev>`
```

Task 11 replaces this missing-version error with lazy materialization.

- [ ] **Step 6: Update help and argument coverage**

Add `[--at REV]` to query, path, and explain help. Extend `coverage_paths.rs` with missing `--at` values and `--graph`/`--at` conflicts for Compass.

- [ ] **Step 7: Run historical query equivalence tests and commit**

Run: `cargo test -p compass-cli --test history_cli && cargo test -p compass-query`

```bash
git add crates/compass-core/src/lib.rs crates/compass-cli
git commit -m "feat(query): load graph history by commit"
```

### Task 10: Add exact-commit worktrees and reusable materialization orchestration

**Files:**
- Modify: `compass/crates/compass-history/Cargo.toml`
- Modify: `compass/crates/compass-history/src/git.rs`
- Modify: `compass/crates/compass-history/tests/git.rs`
- Modify: `compass/crates/compass-files/src/detect.rs`
- Modify: `compass/crates/compass-files/tests/contracts.rs`
- Modify: `compass/crates/compass-core/Cargo.toml`
- Create: `compass/crates/compass-core/src/history.rs`
- Modify: `compass/crates/compass-core/src/lib.rs`
- Create: `compass/crates/compass-core/tests/history_materialize.rs`

**Interfaces:**
- Consumes: `Repository`, `HistoryStore`, `MaterializeRequest`, and `CompleteGraphBuilder`.
- Produces: `WorktreeGuard`, `materialize_history`, ancestor seeding, and an injectable build boundary.

- [ ] **Step 1: Write failing Git worktree cleanup tests**

```rust
#[test]
fn detached_worktree_is_exact_and_removed_on_drop() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = repository_with_two_commits()?;
    let repository = Repository::discover(fixture.path())?;
    let first = repository.resolve("HEAD~1")?;
    let checkout_path;
    {
        let checkout = repository.detached_worktree(&first)?;
        checkout_path = checkout.path().to_path_buf();
        assert_eq!(repository.resolve_at(checkout.path(), "HEAD")?, first);
        assert!(checkout.path().join("old.rs").is_file());
    }
    assert!(!checkout_path.exists());
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify worktree APIs are absent**

Run: `cargo test -p compass-history --test git detached_worktree_is_exact_and_removed_on_drop`

Expected: FAIL with missing `detached_worktree`.

- [ ] **Step 3: Implement `WorktreeGuard` without touching the user's checkout**

Add `tempfile.workspace = true` to regular `compass-history` dependencies. Before checkout,
inventory configured filter drivers and return a typed limitation for unsupported external
smudge/process drivers. Create the temporary directory below `<common-dir>/compass/tmp/`,
then run Git with hooks disabled, `GIT_LFS_SKIP_SMUDGE=1`, `GIT_TERMINAL_PROMPT=0`, no
credential prompting, and no fetch command:

```text
git -c core.hooksPath=<empty-hooks-dir> -C <repository-root> worktree add --quiet --detach <path> <full-commit>
```

`WorktreeGuard::drop` runs `git worktree remove --force -- <path>` and then removes the temporary directory. Add an explicit `close(self) -> Result<(), HistoryError>` used by success paths so cleanup failures are reportable; `Drop` remains a best-effort fallback. Verify exact `HEAD`, preserve committed LFS pointer files, report LFS pointers/gitlinks, and fail on missing objects without network access. Add an explicit historical ignore policy to `compass-files` that applies committed `.gitignore`/`.graphifyignore` files and explicit excludes but skips `.git/info/exclude` and global ignores; the history builder selects it without changing current-checkout defaults. Add tests proving checkout hooks do not run and caller-local ignore state does not affect the historical corpus.

Before every worktree remove or recursive cleanup, revalidate that the stored path is a
nonsymlink descendant of the canonical `<common-dir>/compass/tmp` directory and matches the
random directory created by this guard. Refuse cleanup on any mismatch. Tests replace path
components with symlinks and prove no path outside `tmp` is touched.

- [ ] **Step 4: Write a failing fake-builder materialization test**

```rust
#[test]
fn materializer_reuses_preferred_ancestor_and_publishes_target() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = materialization_fixture()?;
    let builder = RecordingBuilder::default();
    let published = materialize_history(&fixture.store, &builder, fixture.request(false))?;
    assert_eq!(published.version.git_commit, fixture.target.to_string());
    assert_eq!(builder.calls(), 1);
    assert_eq!(builder.seed_commits(), vec![fixture.parent.to_string()]);
    assert_eq!(fixture.store.preferred(&fixture.target)?.unwrap().id, published.id);
    Ok(())
}
```

- [ ] **Step 5: Define the build boundary and request**

```rust
pub trait CompleteGraphBuilder {
    fn build(
        &self,
        checkout: &Path,
        output_root: &Path,
        seed: Option<&GraphArtifacts>,
    ) -> Result<CompletedGraphArtifacts, MaterializeError>;
}

pub struct MaterializeRequest {
    pub repository: Repository,
    pub commit: CommitId,
    pub profile: BuildProfile,
    pub rebuild: bool,
    pub replace_corrupt: bool,
}
```

`MaterializeError` wraps history errors, builder diagnostics, worktree cleanup failure, and incomplete graph state without flattening them into strings.

Provide `materialize_history` with a no-op observer and
`materialize_history_with_observer` for workers. The latter accepts
`&mut dyn MaterializeObserver` and reports `Building`, `Validating`, and `Publishing`
immediately before those phases; an observer failure stops before the next phase.

- [ ] **Step 6: Implement first-parent seed selection**

After creating the exact worktree, compute the final `ExtractionFingerprint` from the profile plus target-commit configuration, committed `.gitignore`/`.graphifyignore` contents, explicit excludes, and all declared meaning-affecting versions/settings. Walk `git rev-list --first-parent <target>` from the target's parent toward the root. Select the first commit with a preferred validated realization whose extraction fingerprint equals that resolved fingerprint. Reconstruct its artifacts and extraction manifest and pass them to the builder. If no compatible ancestor exists, pass `None`.

- [ ] **Step 7: Implement materialization state transitions**

`materialize_history` returns the existing validated preferred version when one exists and `rebuild` is false. Otherwise it acquires the shared activity guard, creates the worktree, resolves the fingerprint, reports/builds, verifies `built_at_commit` equals the requested commit, cross-checks `CompletionEvidence` against the exact-worktree semantic inventory and manifest, reports/validates the complete artifacts, and calls `publish_with_activity` with `make_preferred: true` before closing the worktree and returning `PublishedVersion`. Builder failure or incomplete artifacts never call publication. A corrupt preferred leaves an ordinary build/rebuild candidate non-preferred; only `replace_corrupt: true` invokes the exact-CAS recovery API.

- [ ] **Step 8: Run history/core tests and commit**

Run: `cargo test -p compass-history --test git && cargo test -p compass-core --test history_materialize && cargo clippy -p compass-core -p compass-history --all-targets -- -D warnings`

```bash
git add crates/compass-history crates/compass-files crates/compass-core
git commit -m "feat(history): materialize exact Git commits"
```

### Task 11: Implement `compass history build`, `compass history rebuild`, and lazy query backfill

**Files:**
- Create: `compass/crates/compass-cli/src/history_build.rs`
- Modify: `compass/crates/compass-cli/src/history_commands.rs`
- Modify: `compass/crates/compass-cli/src/lib.rs`
- Modify: `compass/crates/compass-cli/tests/history_cli.rs`
- Modify: `compass/crates/compass-semantic/src/lib.rs`

**Interfaces:**
- Consumes: `materialize_history`, current executable, semantic prompt, provider/model flags, and optional ancestor seed.
- Produces: production `CompleteGraphBuilder`, `compass history build/rebuild`, and automatic materialization for `--at`.

- [ ] **Step 1: Write a failing lazy-backfill integration test**

```rust
#[test]
fn missing_code_only_commit_is_built_on_first_query() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = pure_code_repository_with_two_commits()?;
    let output = fixture.run_compass(&["query", "OldService", "--at", "HEAD~1"])?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("OldService"));
    let status = fixture.run_compass(&["history", "status", "HEAD~1"])?;
    assert!(status.status.success());
    Ok(())
}
```

The fixture contains only deterministic source files, so a complete graph requires no provider credentials while still exercising the normal complete extraction path.

- [ ] **Step 2: Run the test and verify the missing-version error**

Run: `cargo test -p compass-cli --test history_cli missing_code_only_commit_is_built_on_first_query`

Expected: FAIL with `no graph realization`.

- [ ] **Step 3: Expose the semantic prompt fingerprint**

Add:

```rust
pub fn extraction_prompt_sha256(deep: bool) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(extraction_prompt(deep).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
```

Test the shallow and deep digests differ and remain 64 lowercase hex characters.

- [ ] **Step 4: Parse `compass history build` inputs and construct the build profile**

Accept repository-local meaning inputs: `--backend`, `--model`, `--mode`, `--cargo`,
`--dedup-llm`, `--token-budget`, `--resolution`, `--exclude-hubs`, `--no-gitignore`,
and repeated `--exclude`. Reject `--allow-partial`, `--code-only`, and `--no-cluster`
because a history realization must be complete. Reject mutable external-source flags such
as `--google-workspace` and `--postgres` in this first release. Include normalized
values, Compass version, graph schema, extractor/resolver/pipeline versions, enabled
features, prompt digest, direction, and clustering settings in `BuildProfile`. After the
exact worktree exists, add hashes of target-commit configuration and every applied committed
`.gitignore`/`.graphifyignore` plus explicit excludes to `ExtractionFingerprintInput`.
Never read `.git/info/exclude` or global ignores. Record runtime-only
limits such as concurrency and timeout in job metadata, not in the fingerprint. Never
include environment credential values.

- [ ] **Step 5: Implement the current-executable builder**

`NativeCompleteGraphBuilder` writes the optional seed to `<output-root>/graphify-out`, then invokes the current executable with `current_dir(checkout)`, `GRAPHIFY_SKIP_HOOK=1`, and the selected build flags:

```rust
let mut command = std::process::Command::new(&self.executable);
command
    .arg("extract")
    .arg(checkout)
    .arg("--out")
    .arg(output_root)
    .arg("--no-viz")
    .current_dir(checkout)
    .env("GRAPHIFY_SKIP_HOOK", "1");
```

Forward the normalized semantic flags, wait for completion, and retain bounded
stdout/stderr diagnostics. On exit `0`, return `CompletedGraphArtifacts` by deriving
`CompletionEvidence` from the build
result and manifest: count the semantic corpus and completed semantic entries, record
failed chunks, and assert partial mode was not enabled. Pass that evidence while
loading `GraphArtifacts`; reject a graph whose `built_at_commit` is not the requested
full commit. Never infer completeness from `.graphify_semantic_marker`, because that
file records token use rather than success. Raw marker bytes, `cost.json`, timings, and
diagnostics stay in bounded attempt provenance and never enter the five identity-bearing trees.

- [ ] **Step 6: Wire synchronous build and rebuild commands**

`compass history build <rev>` resolves the commit and returns an existing preferred version when present. `compass history rebuild <rev>` always runs a new extraction attempt. `compass history rebuild <rev> --replace-corrupt` is the only path that may exact-CAS a validated candidate over a corrupt preferred realization; reject the flag when the preferred realization is not corrupt. Both may create the SQLite store while eager history remains disabled. Both print commit, realization ID, fingerprint, all five counts, and whether the result became preferred; `--format json` emits the same fields as one stable object.

- [ ] **Step 7: Replace missing-version errors with lazy materialization**

Add one `resolve_or_materialize(revision, options)` path used by query/path/explain
`--at`, both sides of `compass diff`, and `compass history build/rebuild/export`. `compass history status`, `compass history list`, and `compass history show` remain
inspection-only and never extract. The service invokes the same
materializer with normalized default extraction settings when no preferred realization
exists. Display build progress on stderr; preserve command output on stdout. A semantic
provider error returns exit code `1`; Task 12 adds durable failed-job diagnostics, and
no failure creates a preferred version.

The service calls `HistoryStore::open_existing` first and `HistoryStore::create` only after it
has established that materialization is required. It never changes the eager enablement flag.

- [ ] **Step 8: Test ancestor seeding, rebuild, failure, and merge commits**

Add CLI fixtures that assert:

- a second historical build seeds from the compatible first parent;
- `rebuild` keeps the old realization addressable;
- provider failure leaves no preferred root;
- a merge commit graph contains files from the exact merge tree;
- diffing two previously unseen revisions lazily materializes both before streaming;
- uncommitted files in the user's checkout never appear in the historical graph.
- committed ignore files and explicit excludes affect the fingerprint and corpus, while
  `.git/info/exclude` and global ignores do not;
- checkout hooks, LFS smudging, credential prompts, network fetching, and unsupported
  external filters never execute;
- ordinary rebuild cannot replace corrupt preferred state, while explicit
  `--replace-corrupt` performs exact-CAS recovery.

- [ ] **Step 9: Run integration tests and commit**

Run: `cargo test -p compass-cli --test history_cli && cargo test -p compass-core --test history_materialize && cargo clippy -p compass-cli -p compass-core -p compass-history --all-targets -- -D warnings`

```bash
git add crates/compass-cli crates/compass-semantic/src/lib.rs
git commit -m "feat(history): lazily build complete commit graphs"
```

### Task 12: Queue eager post-commit builds and persist job state

**Files:**
- Modify: `compass/Cargo.toml`
- Modify: `compass/Cargo.lock`
- Modify: `compass/crates/compass-history/Cargo.toml`
- Create: `compass/crates/compass-history/src/config.rs`
- Create: `compass/crates/compass-history/src/durable.rs`
- Create: `compass/crates/compass-history/src/jobs.rs`
- Create: `compass/crates/compass-history/src/leases.rs`
- Modify: `compass/crates/compass-history/src/lib.rs`
- Create: `compass/crates/compass-history/tests/jobs.rs`
- Modify: `compass/crates/compass-cli/src/history_commands.rs`
- Modify: `compass/crates/compass-cli/src/hook_commands.rs`
- Modify: `compass/crates/compass-cli/src/lib.rs`
- Modify: `compass/crates/compass-cli/tests/hook_cli.rs`
- Modify: `compass/crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Consumes: full commit IDs, normalized materialization options, and the shared history store.
- Produces: `HistoryConfig::{load, enable, disable}`, `HistoryQueue::{enqueue, claim_or_join, transition, reconcile, list}`, `LeaseGuard`, hidden `compass history-worker`, and opt-in non-blocking eager builds.

- [ ] **Step 1: Write failing durable queue transition tests**

```rust
#[test]
fn jobs_follow_the_allowed_state_machine_and_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let queue = HistoryQueue::open(directory.path())?;
    let id = queue.enqueue(job_request())?;
    let claimed = queue.claim_next()?.unwrap();
    assert_eq!(claimed.id, id);
    assert_eq!(claimed.state, JobState::Building);
    queue.transition(&id, JobState::Validating, None)?;
    queue.transition(&id, JobState::Published, None)?;
    drop(queue);
    assert_eq!(HistoryQueue::open(directory.path())?.get(&id)?.unwrap().state, JobState::Published);
    Ok(())
}
```

Add tests rejecting `queued -> published` and `failed -> building`, plus simultaneous join,
heartbeat, expiration/reclaim, generation increment, late-worker rejection, and reopen.
Also test absent configuration, enable/disable idempotency, invalid-profile rollback, and that
read-only status creates neither `config.json` nor the SQLite/operational paths.

- [ ] **Step 2: Run the queue tests and verify the module is missing**

Run: `cargo test -p compass-history --test jobs`

Expected: FAIL with unresolved queue types.

- [ ] **Step 3: Implement explicit job records and transitions**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState { Queued, Building, Validating, Published, Failed, Incomplete }

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct JobRecord {
    pub id: String,
    pub commit: CommitId,
    pub profile: BuildProfile,
    pub profile_digest: String,
    pub resolved_fingerprint: Option<String>,
    pub state: JobState,
    pub attempts: u32,
    pub diagnostic: Option<String>,
    pub candidate_realization: Option<RealizationId>,
    pub observed_preferred: Option<RealizationId>,
    pub preferred: Option<bool>,
    pub lease_generation: u64,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
}
```

Store one bounded canonical JSON record per attempt under `<common-dir>/compass/jobs` using a
shared writer from `durable.rs`. It creates an owner-only temporary file in the same directory,
flushes and `sync_all`s it, atomically replaces the destination, and synchronizes the parent
directory. Unix uses atomic rename. Windows uses `MoveFileExW` with
`MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH` through an exact-pinned target-specific
`windows-sys = "=0.61.2"` dependency, or a proven standard-library equivalent if available at
implementation time. Reject symlink targets and fail closed when this contract is unavailable.
Never use the existing copy-over fallback in `compass-files::atomic` for job or lease state. Add
crash/fault-injection tests before and after every sync/replace boundary.

Reject a job before allocation or decoding if it exceeds `MAX_JOB_BYTES`; cap redacted
diagnostics at `MAX_DIAGNOSTIC_BYTES`. Queue deduplication uses commit plus profile digest
because the final fingerprint is unavailable before exact checkout. After checkout, persist
the resolved fingerprint and verify binary/pipeline versions against prior attempts.
Credentials and credential values never enter any persisted field.

`leases.rs` stores `leases/<commit>-<profile-digest>.lease` with a random owner ID,
generation, and expiration. A lease lasts 120 seconds and heartbeats every 30 seconds. Claiming
creates or compare-and-swaps the lease; heartbeat refreshes only the same owner/generation;
reclaim increments generation after expiration; every job write checks generation so a late
worker fails. Wall-clock jumps may duplicate extraction work but cannot overwrite a newer
generation or corrupt the catalog. Lazy and eager requests join the same live lease, while
explicit rebuild creates a new attempt.

- [ ] **Step 4: Add draining worker execution**

The hidden `compass history-worker` command first reconciles stale attempts, then repeatedly claims the
oldest queued job and runs `materialize_history_with_observer` until no queued job remains.
Multiple launched workers join or skip live leases safely. A terminal `Failed` or `Incomplete`
attempt is recorded and does not prevent later jobs from running. Its observer maps `Building` and `Validating` to
the durable queue states; `Publishing` is kept as `Validating` because the public state
machine has no externally observable half-published state. The worker then records
`Published`, `Failed`, or `Incomplete`. Diagnostics are capped at 64 KiB and redact
environment values matching credential variable names. Before catalog publication, persist
the candidate realization ID and observed preferred ID. On startup, `reconcile` checks stale
jobs against the immutable catalog: if the candidate exists, retry preferred CAS only when
the observed preferred remains exact; otherwise record `Published { preferred: false }`.
Test termination before catalog publication, between catalog/preferred CAS, and between
preferred CAS/terminal persistence. Also test that one launch after several queued commits
drains all of them in FIFO claim order, including when an earlier job fails.

- [ ] **Step 5: Add explicit enablement and enqueue the exact post-commit SHA**

Implement `compass history enable [build-profile options]` and `compass history disable`. `enable` validates and atomically stores a normalized non-secret eager profile
in `<common-dir>/compass/config.json`, creates and verifies the SQLite store, and installs or
updates Compass's managed post-commit hook block through the existing hook installer while
preserving all user-owned hook content. If hook installation fails, enablement rolls back.
`disable`
atomically marks eager history disabled, is idempotent, retains the database/jobs/leases, and
does not kill an already leased worker. It leaves the inert managed block installed so current
graph refresh behavior and later re-enablement remain stable. Explicit builds and lazy
materialization remain usable while disabled. Invalid enable options leave the previous
configuration unchanged.

Change only the managed post-commit hook block to check the enabled configuration, resolve
`git rev-parse --verify HEAD^{commit}`, and invoke the current frontend's hidden spawn command:

```text
compass hook-spawn . --history-commit <full-sha>
```

When history is disabled or has never been configured, the block performs no history enqueue.
When enabled, it resolves and enqueues the SHA before refresh-only guards for
rebase, merge, cherry-pick, changed-file filtering, graph-only commits, or linked worktrees.
Those guards may suppress rebuilding current `graphify-out`, but they never suppress history
enqueueing. `compass hook-spawn` launches the frontend-correct `compass history-worker` in the background with the existing
detached-process policy and independently launches current graph refresh when required. The
post-checkout hook continues refreshing the current graph but does not create a new commit job.

- [ ] **Step 6: Make `compass history status` include jobs**

Always display eager enablement and the normalized profile digest without secrets. When no
store exists, report `disabled` and `no store` with exit `0` without creating paths. When no
preferred realization exists, display the newest job's state, attempt count, and bounded diagnostic. When a preferred version exists, show it first and list any newer failed rebuild attempts separately. Always report store-format compatibility and detected target limitations such as LFS pointers, gitlinks, or unsupported filters.

- [ ] **Step 7: Test hook latency and exact SHA capture**

In `hook_cli.rs`, install the hook and first prove a commit while disabled creates no job. Enable
history, commit a pure-code change, and assert the commit command completes without waiting for
the worker. Poll with a bounded test deadline for the job file, then assert its commit equals the
new full SHA. Disable again and prove stored realizations remain queryable while later commits
are not queued. Repeat from a linked worktree and for merge/cherry-pick commit state. Use a fake
executable for provider-failure and launch-failure paths, then launch one later worker and prove
it drains all queued jobs.

- [ ] **Step 8: Run queue/hook tests and commit**

Run: `cargo test -p compass-history --test jobs && cargo test -p compass-cli --test hook_cli && cargo test -p compass-cli --test history_cli`

```bash
git add Cargo.toml Cargo.lock crates/compass-history crates/compass-cli
git commit -m "feat(history): enqueue complete graphs after commits"
```

### Task 13: Add reachability-based garbage collection and retention controls

**Files:**
- Create: `compass/crates/compass-history/src/gc.rs`
- Modify: `compass/crates/compass-history/src/store.rs`
- Modify: `compass/crates/compass-history/src/lib.rs`
- Modify: `compass/crates/compass-history/tests/publication.rs`
- Create: `compass/crates/compass-history/tests/maintenance.rs`
- Modify: `compass/crates/compass-cli/src/history_commands.rs`
- Modify: `compass/crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Consumes: all named roots, preferred realization set, Prolly `NamedRootRetention`, and job retention policy.
- Produces: `HistoryStore::{plan_gc, sweep_gc}` and `compass history gc [--prune-non-preferred] [--yes]`.

- [ ] **Step 1: Write failing orphan and retention tests**

```rust
#[test]
fn gc_keeps_all_published_versions_and_removes_orphans() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let history = fixture_with_preferred_and_nonpreferred(directory.path())?;
    history.write_unpublished_fixture_tree()?;
    let plan = history.plan_gc(false)?;
    assert!(plan.reclaimable_nodes > 0);
    assert_eq!(plan.prunable_realizations, 0);
    let sweep = history.sweep_gc(plan)?;
    assert!(sweep.deleted_nodes > 0);
    assert_eq!(history.list(None)?.len(), 2);
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify GC APIs are missing**

Run: `cargo test -p compass-history --test publication gc_keeps_all_published_versions_and_removes_orphans`

Expected: FAIL with missing `plan_gc`.

- [ ] **Step 3: Plan GC from all retained named roots**

Acquire the exclusive `maintenance.lock` guard before reading any roots and retain it through
planning, catalog recheck, root removal, node deletion, and cleanup. For normal GC, call:

```rust
let retention = prolly::NamedRootRetention::all();
let plan = self.prolly.plan_store_gc_for_retention(&retention)?;
```

Wrap the Prolly plan with job/temp cleanup counts. Normal GC never removes a published realization root. Add separate-process tests proving an active shared reader or builder blocks/times out maintenance and that GC cannot delete an active builder's unpublished nodes.

- [ ] **Step 4: Plan non-preferred pruning explicitly**

Compute the preferred realization IDs, identify other published IDs, and list the six named roots that would be removed for each. The plan is immutable and includes a digest of the observed catalog state. `sweep_gc` rechecks that digest before deleting roots in a strict transaction, then calls Prolly's store GC against all remaining named roots.

- [ ] **Step 5: Add safe CLI behavior**

- `compass history gc` prints a plan and sweeps abandoned objects plus expired temp/job state.
- `compass history gc --prune-non-preferred` prints the realization IDs but does not delete them without `--yes`.
- `compass history gc --prune-non-preferred --yes` applies the checked plan.
- Never accept a broad filesystem path or glob as a GC target.
- Report deleted SQLite node rows and reusable bytes/pages when available; never claim the
  physical `history.sqlite` file shrank.
- Retain terminal attempt records for 30 days by default. Remove temporary directories only
  after 24 hours and only when no live lease owns them. Never delete queued or active attempts.
  Validate every cleanup candidate as a nonsymlink descendant of the canonical common-dir
  `compass/tmp` or `compass/jobs` directory.
- `--format json` emits the immutable plan and applied result as stable objects suitable for
  automation; progress and confirmation prompts remain on stderr.

- [ ] **Step 6: Test stale plans and concurrent preferred changes**

Create a plan, change the preferred version, then assert sweep refuses the stale plan and removes nothing. Add tests for corrupt root listings, interrupted root deletion rollback, shared-reader/builder races, maintenance timeout, 30-day terminal-attempt retention, 24-hour temp retention, live-lease preservation, symlink/path-escape rejection, and the store-format root remaining retained.

- [ ] **Step 7: Run tests and commit**

Run: `cargo test -p compass-history && cargo test -p compass-cli --test history_cli && cargo clippy -p compass-history -p compass-cli --all-targets -- -D warnings`

```bash
git add crates/compass-history crates/compass-cli
git commit -m "feat(history): garbage collect unreachable graph objects"
```

### Task 14: Qualify semantic correctness, coverage, and performance; document the feature

**Files:**
- Create: `compass/crates/compass-history/tests/performance.rs`
- Modify: `compass/crates/compass-cli/tests/history_cli.rs`
- Modify: `compass/README.md`
- Refresh (generated, normally ignored): `graphify-out/` through `compass update .`

**Interfaces:**
- Consumes: the complete feature and existing graph/query compatibility oracle.
- Produces: semantic-equivalence evidence, structural-sharing measurements, user documentation, and a fresh Compass project graph.

- [ ] **Step 1: Add graph export and query semantic-equivalence coverage**

Add a fixture that builds a normal Rust graph, publishes it, reconstructs it, and compares normalized JSON with the original. Run query, path, and explain against both `--graph` and `--at HEAD`; assert identical stdout after excluding only query-stamp side effects.

- [ ] **Step 2: Add an ignored performance evidence test**

```rust
#[test]
#[ignore = "performance evidence; run explicitly"]
fn small_change_reuses_content_addressed_nodes() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = compass_sized_fixture()?;
    let first = fixture.publish_base()?;
    let before = fixture.sqlite_node_row_count()?;
    let second = fixture.publish_one_file_change()?;
    let after = fixture.sqlite_node_row_count()?;
    let diff = fixture.structural_diff(&first, &second)?;
    assert!(diff.shared_nodes > 0);
    assert!(after - before < diff.second_total_nodes);
    Ok(())
}
```

Obtain logical counts through `NodeStoreScan` or a counting adapter, never adapter-private SQL or filesystem object enumeration. Print cold/seeded publication time, SQLite logical growth and reusable pages, multi-process contention, diff latency and peak buffered records, query latency/memory, and GC planning/sweep. Record evidence without inventing a percentage gate not present in the design.

- [ ] **Step 3: Add malformed and adversarial correctness cases**

Cover legacy `edges`, unknown attributes, multigraph parallel edges, duplicate id-less hyperedges, canonical number fixtures, artifact registry failures, every exact resource-limit boundary without large allocation, Unicode IDs, long relations, corrupt manifests/preferred recovery, missing Git, SHA-1/SHA-256, shallow history, linked worktrees, concurrent SQLite connections, WAL/busy timeout, old fixture reopen, stale leases/late workers, clock jumps, crash reconciliation windows, queue draining after failed jobs, maintenance races, ignored caller-local excludes, LFS/gitlink/filter reporting, symlink/path-escape rejection, no-store read-only behavior, enable/disable behavior, and Compass-specific help and process contracts.

- [ ] **Step 4: Document the command lifecycle**

Update `compass/README.md` with:

```text
compass history enable
compass history build HEAD
compass query "authentication flow" --at HEAD~20
compass diff v1.2.0 HEAD --detailed
compass history export HEAD --format graphify-out --output <output-directory>
compass history list HEAD --format json
compass history gc
compass history disable
```

Do not promise Graphify command compatibility. If the legacy binary is mentioned, label it an
optional best-effort transition shim outside the Compass versioned-graph contract.
Explain explicit enable/disable semantics, complete semantic graphs, extraction fingerprints,
multiple realizations, the live-WAL SQLite store under the Git common directory, owner-only
permissions, eager queue/lease behavior, lazy backfill, provider requirements,
canonical-semantic export and versioned derived regeneration, committed-ignore/filter
limitations, explicit `--replace-corrupt` recovery, exact exit/output behavior, and SQLite GC's
logical rather than physical reclamation.

- [ ] **Step 5: Run formatting, focused tests, and workspace lint**

Run:

```bash
cargo fmt --all --check
cargo test -p compass-history
cargo test -p compass-core --test history_materialize
cargo test -p compass-cli --test history_cli
cargo test -p compass-cli --test hook_cli
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: every command exits `0`.

- [ ] **Step 6: Run the complete workspace and coverage gates**

Run:

```bash
cargo test --workspace --all-features --locked
GRAPHIFY_REPO_ROOT=/Users/haipingfu/graphify GRAPHIFY_PYTHON=/Users/haipingfu/graphify/.venv/bin/python cargo llvm-cov --workspace --all-features --all-targets --locked --exclude compass-tree-sitter-language-pack --lcov --output-path target/compass.lcov
cargo llvm-cov report --summary-only --fail-under-lines 90 --fail-under-regions 85
scripts/check_critical_coverage.sh target/compass.lcov 95
```

Expected: tests pass; workspace line coverage is at least 90%, region coverage at least 85%, and critical-module line coverage at least 95%.

- [ ] **Step 7: Run the explicit structural-sharing evidence test**

Run: `cargo test -p compass-history --test performance small_change_reuses_content_addressed_nodes -- --ignored --nocapture`

Expected: PASS and print the recorded storage/diff measurements.

- [ ] **Step 8: Refresh Compass's project graph**

Run: `compass update .`

Expected: `graphify-out/GRAPH_REPORT.md` records the implementation commit and the graph passes its health checks.

Do not force-add ignored generated outputs. Commit them only if `git ls-files
graphify-out` shows they were already tracked.

- [ ] **Step 9: Commit qualification and documentation**

```bash
git add README.md crates/compass-history/tests/performance.rs crates/compass-cli/tests/history_cli.rs
git commit -m "docs(history): qualify versioned graph maps"
```

When executing from the Graphify superproject, advance its `compass` submodule pointer only
after the reviewed Compass branch is pushed. That integration commit is separate from the
Compass task commits above; a standalone Compass checkout has no parent-pointer step.

## Requirement traceability

| Production requirement | Owning tasks | Required evidence |
|---|---:|---|
| Exact pinned SQLite Prolly storage and linked-worktree sharing | 1, 4, 13 | adapter contract, reopen fixture, WAL contention, strict transactions, GC |
| Stable identity, typed keys, complete lossless graph and artifact registry | 2, 3, 5 | golden bytes, round-trip/property tests, corruption and resource-limit tests |
| Atomic immutable publication and explicit corrupt recovery | 4, 5, 11, 12 | CAS races, fault windows, `--replace-corrupt` tests |
| Streaming topology/attribute diff | 6, 8, 11 | differential oracle, equal-root skip, bounded/failing sinks |
| Compass-first query/path/explain at any commit | 7, 9, 11 | exact-revision semantic-equivalence and lazy-backfill tests |
| Canonical export plus versioned report/HTML regeneration | 3, 7, 14 | authoritative byte equality and renderer-version compatibility tests |
| Exact offline Git-tree materialization | 10, 11 | merge/shallow/filter/ignore/LFS/gitlink and cleanup-safety tests |
| Explicit opt-in eager generation and durable recovery | 12 | enable/disable, hook latency, durable-write fault injection, lease and queue-drain tests |
| Safe retention and maintenance | 1, 5, 13 | shared/exclusive process races, stale plans, lease-aware retention |
| Fully usable Compass command surface | 7–14 | Compass help/usage, `--`, JSON, stdout/stderr, and exit-code matrix |
| Release readiness without default-on regression | 14 | format, full tests, coverage gates, performance evidence, refreshed project graph |

## Final verification checklist

- [ ] Every preferred commit graph contains semantic, inferred, and hyperedge data.
- [ ] Rebuilding the same commit, parents, fingerprint, persisted formats, and canonical graph content yields the same realization ID regardless of raw token/cost/timing provenance.
- [ ] Different fingerprints and nondeterministic outputs coexist without overwrite.
- [ ] `--at` always reads the exact committed tree and ignores uncommitted files.
- [ ] Graph diffs stream Prolly changes and skip equal roots.
- [ ] Partial or corrupt output cannot become preferred.
- [ ] Post-commit work is queued and does not block the commit.
- [ ] Post-commit enqueue happens only after explicit `compass history enable`; disable is idempotent, retains history, and does not affect explicit/lazy builds.
- [ ] Linked worktrees share one store through the Git common directory.
- [ ] The store is `history.sqlite` with exact adapter pins, WAL/full synchronous settings, busy timeout, owner-only protection, and verified `compass/store-format/v1`.
- [ ] Directed/undirected simple and multigraph structures, exact duplicate id-less hyperedges, order, unknown fields, and authoritative sidecars round-trip.
- [ ] Historical builds ignore `.git/info/exclude`/global ignores and never execute hooks, LFS smudging, credential prompts, network fetches, or unsupported filters.
- [ ] Durable generation leases reject late workers and reconcile every catalog/preferred/job crash window.
- [ ] A worker reconciles and drains all queued jobs, even when an earlier attempt fails.
- [ ] Operational records use owner-only, symlink-safe, crash-durable atomic replacement; no copy-over fallback can masquerade as atomicity.
- [ ] One reader-writer maintenance lock prevents GC races with readers, builders, reconstruction, diff, and publication.
- [ ] Corrupt preferred state is replaced only through explicit `--replace-corrupt` exact-CAS recovery.
- [ ] Normal GC retains every published realization.
- [ ] `graph.json` and authoritative sidecars reconstruct with canonical semantic equivalence; reports/HTML regenerate from recorded versions.
- [ ] No credentials or machine-specific paths appear in fingerprints, manifests, or jobs.
- [ ] `compass` is the sole canonical command surface and satisfies its documented help and process contracts independently of any legacy Graphify shim.
- [ ] Read-only no-store commands create nothing; stdout/stderr, JSON shape, and exit codes satisfy the public process contract.
- [ ] All exact v1 size/count/depth and retention limits have boundary tests.
