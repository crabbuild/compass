# Django Sub-Five-Second Indexing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make cold Django initialization, forced full indexing, and unchanged updates each complete in less than five seconds while preserving deterministic graph and Program IR outputs and reporting built-in wall time.

**Architecture:** Build one reusable source snapshot, feed one tree-sitter parse into graph and Program extraction, use compact versioned internal caches, and overlap independent graph/Program work. Seal completed artifacts with digests and saved statistics so unchanged updates verify bytes without parsing hundreds of megabytes of JSON.

**Tech Stack:** Rust 2024, tree-sitter, Rayon, serde, serde_json, rmp-serde, sha2, Bash, Python 3 benchmark helper

## Execution style

Implement each task as a coherent change, then add or update regression coverage
and run the listed verification. Do not intentionally stage failing tests before
the implementation. Every task still ends with correctness evidence,
performance evidence where relevant, and an independently reviewable commit.

## Baseline and acceptance target

Release measurements on the current 12-core Apple Silicon machine and Django
checkout:

| Operation | Wall time | Evidence |
| --- | ---: | --- |
| Cold successful init/build | 21.08s | 6,051 files |
| Unchanged update | 5.83s | 6,051 cached files |
| Cold no-cluster extraction | 17.41s | 52,190 nodes, 197,340 edges |

Cold no-cluster work currently costs approximately:

- detection: 2.5s;
- duplicated Program and graph extraction: 10.1s;
- Program/graph serialization and writes: 4.5s.

Loading and validating the existing 272 MB `program.json` alone costs 5.22s,
which explains almost all unchanged-update latency.

Completion requires three samples of each command, with every sample below
5.0s:

```bash
compass init . --yes --force --timing
compass update . --force --timing
compass update . --timing
```

## Global constraints

- Public `graph.json` and `program.json` remain deterministic JSON.
- Graph facts, Program IR, clustering, reports, caches, and state are complete
  before the command exits.
- Production indexing adds no Python process, network service, background
  daemon, or Django-specific branch.
- Explicit `--force` never uses the unchanged fast path.
- Corrupt, truncated, stale, option-mismatched, or interrupted output falls
  back safely to validation or rebuilding.
- A cold deterministic build reads each source at most once and parses every
  Program-supported tree-sitter source at most once.
- Source edits preserve incremental AST and Program syntax reuse.
- Performance evidence uses a release binary and the fixed Django commit.
- Do not run `graphify update .` as part of this work; qualification evidence
  comes from the real Compass init/update commands below.

## File and responsibility map

- `crates/compass-languages/src/combined.rs`
  - combined graph and Program extraction from one syntax tree.
- `crates/compass-files/src/detect.rs`
  - detection result plus reusable bytes read during cold word counting.
- `crates/compass-files/src/hash.rs`
  - parallel stat-index word counting with optional captured bytes.
- `crates/compass-files/src/cache.rs`
  - versioned MessagePack cache transport and JSON migration reads.
- `crates/compass-files/src/atomic.rs`
  - atomic writes that return byte length and SHA-256.
- `crates/compass-core/src/deterministic.rs`
  - one-read source snapshots and cache-aware combined extraction.
- `crates/compass-core/src/build_state.rs`
  - completed-generation schema and artifact verification.
- `crates/compass-core/src/program.rs`
  - Program assembly from prepared evidence and single output serialization.
- `crates/compass-core/src/publish.rs`
  - concurrent, guarded artifact/cache publication.
- `crates/compass-core/src/pipeline.rs`
  - orchestration, graph/Program concurrency, and stage timing.
- `crates/compass-cli/src/lib.rs`
  - extract/update timing parsing and output.
- `crates/compass-cli/src/init_commands.rs`
  - init timing and reuse of validated detection.
- `scripts/qualify_django_performance.sh`
  - guarded three-workflow acceptance benchmark.

---

### Task 1: Establish the CLI timing contract

**Files:**

- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/init_commands.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/tests/init_cli.rs`
- Modify: `crates/compass-cli/tests/program_cli.rs`

**Interfaces:**

```rust
pub(crate) enum BuildOperation {
    Init,
    Extract,
    Update,
}

pub struct BuildTimings {
    pub detect: Duration,
    pub deterministic_extract: Duration,
    pub graph_assembly: Duration,
    pub program_analysis: Duration,
    pub publish: Duration,
}
```

- [ ] Add `BuildOperation` in the CLI and route init, extract, and update through
  the matching value.

- [ ] Replace the old timing fields with the stable stage model above. Keep
  total time independent because graph, Program, and publication stages will
  overlap later.

- [ ] Accept `--timing` for update by removing the `if extract` parser guard.
  Add `timing: bool` to `InitOptions`, parse `--timing`, and forward it to the
  internal build call.

- [ ] Print exactly one default completion line:

```text
Compass init completed in 4.82s.
Compass extract completed in 4.37s.
Compass update completed in 0.91s.
```

For interactive init, start timing after user confirmation but before scope
detection. On failure, print `Compass <operation> failed after X.XXs.` without a
completion claim.

- [ ] Emit these `--timing` lines on stderr:

```text
[compass timing] detect: 0.4s
[compass timing] deterministic extract: 1.8s
[compass timing] graph assembly: 1.1s
[compass timing] program analysis: 0.9s
[compass timing] publish: 0.8s
[compass timing] total: 4.5s
```

- [ ] Update CLI coverage to assert one parseable elapsed line for each
  operation, stage output for `update --timing`, and failure-duration wording.

- [ ] Verify:

```bash
cargo fmt --all -- --check
cargo test -p compass-cli --test init_cli
cargo test -p compass-cli --test program_cli
cargo test -p compass-cli --lib
```

Expected: all commands accept the documented flag and print exactly one
operation total.

- [ ] Commit:

```bash
git add crates/compass-core/src/pipeline.rs \
  crates/compass-cli/src/lib.rs \
  crates/compass-cli/src/init_commands.rs \
  crates/compass-cli/src/help.rs \
  crates/compass-cli/tests/init_cli.rs \
  crates/compass-cli/tests/program_cli.rs
git commit -m "feat(cli): report index wall time"
```

### Task 2: Seal completed builds and accelerate unchanged updates

**Files:**

- Create: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`
- Modify: `crates/compass-core/tests/pipeline_edge_coverage.rs`

**Interfaces:**

```rust
pub(crate) const BUILD_STATE_FILE: &str = ".compass_build_state.json";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArtifactSeal {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct BuildProfile {
    pub no_cluster: bool,
    pub no_viz: bool,
    pub resolution: f64,
    pub exclude_hubs: Option<f64>,
    pub program_analysis: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SavedStats {
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    pub program_modules: usize,
    pub program_summaries: usize,
    pub program_providers: usize,
    pub program_conflicts: usize,
}
```

- [ ] Implement `ArtifactSeal::capture(path)` using a 1 MB streaming buffer and
  SHA-256. Reject non-regular files.

- [ ] Implement a versioned `BuildState` containing:

  - normalized `BuildProfile`;
  - sealed `manifest.json`;
  - sealed `graph.json`;
  - optional sealed `program.json`;
  - seals for purpose-dependent required outputs;
  - `SavedStats`; and
  - schema string `compass.build-state/1`.

- [ ] Implement:

```rust
pub(crate) fn load_verified(
    output_dir: &Path,
    profile: &BuildProfile,
    manifest_path: &Path,
    prior_build_complete: bool,
) -> Result<Option<BuildState>, CoreError>
```

Return `Ok(None)` for absent, malformed, wrong-schema, profile-mismatched, or
seal-mismatched state. Verification hashes artifacts but never deserializes
`graph.json` or `program.json`.

- [ ] Check `BuildGuard::ensure_complete` before beginning a new guard. A prior
  incomplete marker disables state trust.

- [ ] Replace the unchanged Program JSON load/validation with verified saved
  statistics. Keep the existing full validation path only for migration when
  no state exists.

- [ ] Publish `.compass_build_state.json` last, then commit the build guard.
  Remove stale state before a forced publication begins.

- [ ] Add post-change coverage for:

  - valid unchanged fast path;
  - missing state;
  - unsupported state schema;
  - profile mismatch;
  - truncated and same-size-modified Program output;
  - modified graph output;
  - interrupted-build marker; and
  - successful repair with canonical Program bytes restored.

- [ ] Verify:

```bash
cargo fmt --all -- --check
cargo test -p compass-core --test program_pipeline
cargo test -p compass-core --test pipeline_edge_coverage
cargo test -p compass-core --lib
cargo build --release --locked -p compass-cli --bin compass
/usr/bin/time -p target/release/compass update \
  /Users/haipingfu/Github/django --no-viz --timing
```

Expected: the unchanged update is below 5.0s and Program/graph counts match the
baseline.

- [ ] Commit:

```bash
git add crates/compass-core/src/build_state.rs \
  crates/compass-core/src/lib.rs \
  crates/compass-core/src/pipeline.rs \
  crates/compass-core/tests/program_pipeline.rs \
  crates/compass-core/tests/pipeline_edge_coverage.rs
git commit -m "perf(core): seal completed index generations"
```

### Task 3: Move deterministic caches to MessagePack

**Files:**

- Modify: `crates/compass-files/Cargo.toml`
- Modify: `crates/compass-files/src/cache.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Modify: `crates/compass-files/tests/contracts.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`

**Interface and format:**

```rust
const CACHE_ENCODING_VERSION: u32 = 1;
const MESSAGEPACK_EXTENSION: &str = "msgpack";
```

New deterministic cache directories include `e1`. New records use
`rmp_serde::to_vec_named`; reads try `.msgpack` first and legacy `.json` second.
Public output remains JSON.

- [ ] Add `rmp-serde.workspace = true` to `compass-files`; do not add another
  serialization dependency.

- [ ] Encode AST `Value` records as MessagePack in `save` and `save_batch`.
  Keep repository-relative source rewriting before encoding.

- [ ] Encode Program syntax/artifact records as MessagePack in
  `save_program`. Program values remain repository-relative.

- [ ] Treat malformed MessagePack as a cache miss. A valid legacy JSON entry is
  a migration read, not a permanent write format.

- [ ] Update pruning, clearing, and cache inventory to handle both extensions
  and prefer MessagePack when both exist.

- [ ] Add post-change coverage for AST and Program round trips, legacy reads,
  corrupted entries, concurrent batches, and pruning both extensions.

- [ ] Verify:

```bash
cargo fmt --all -- --check
cargo test -p compass-files --test contracts
cargo test -p compass-core --test program_pipeline
cargo build --release --locked -p compass-cli --bin compass
benchmark_output=$(mktemp -d /tmp/compass-msgpack.XXXXXX)
/usr/bin/time -p target/release/compass extract \
  /Users/haipingfu/Github/django \
  --code-only --force --timing --no-cluster --no-viz \
  --out "$benchmark_output"
du -sh "$benchmark_output/compass-out/cache"
```

Record wall time, cache size, and output counts.

- [ ] Commit:

```bash
git add crates/compass-files/Cargo.toml \
  crates/compass-files/src/cache.rs \
  crates/compass-files/src/lib.rs \
  crates/compass-files/tests/contracts.rs \
  crates/compass-core/tests/program_pipeline.rs
git commit -m "perf(cache): encode deterministic entries with MessagePack"
```

### Task 4: Read and parse each source once

**Files:**

- Create: `crates/compass-languages/src/combined.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/src/program/mod.rs`
- Modify: `crates/compass-languages/tests/program_evidence.rs`
- Modify: `crates/compass-languages/tests/engine_edge_coverage.rs`
- Create: `crates/compass-core/src/deterministic.rs`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/program.rs`
- Modify: `crates/compass-files/src/cache.rs`
- Modify: `crates/compass-files/src/detect.rs`
- Modify: `crates/compass-files/src/hash.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Modify: `crates/compass-files/tests/contracts.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/init_commands.rs`
- Modify: `crates/compass-cli/tests/init_cli.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`

**Language interface:**

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct CombinedExtraction {
    pub graph: Extraction,
    pub program: Option<EvidenceBatch>,
}

impl Engine {
    pub fn extract_source_combined(
        &mut self,
        path: &Path,
        source_file: &str,
        source: &[u8],
    ) -> Result<CombinedExtraction, ExtractError>;
}
```

**Detection and core interfaces:**

```rust
#[derive(Debug, Clone)]
pub struct DetectionSnapshot {
    pub detection: Detection,
    pub source_bytes: BTreeMap<String, Vec<u8>>,
}

pub(crate) struct SourceSnapshot {
    pub path: PathBuf,
    pub source_file: String,
    pub language: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub ast_hash: String,
}

pub(crate) struct DeterministicFileResult {
    pub snapshot: SourceSnapshot,
    pub graph: Extraction,
    pub program: Option<EvidenceBatch>,
    pub graph_reused: bool,
    pub program_reused: bool,
    pub pending_graph_cache: Option<serde_json::Value>,
    pub pending_program_cache: Option<EvidenceBatch>,
}
```

- [ ] Refactor Program extraction so both the standalone provider and combined
  API call:

```rust
pub(crate) fn extract_from_tree(
    source_file: &str,
    language: &'static str,
    source: &[u8],
    root: tree_sitter::Node<'_>,
) -> Result<EvidenceBatch, ProviderError>
```

It must construct the existing provider descriptor, dispatch Python/Rust/JS
extractors, and retain `merge_evidence` validation.

- [ ] Split generic graph extraction at the parsed-tree boundary. For Python,
  Rust, TypeScript, TSX, and JavaScript, parse once and pass the same root to
  graph and Program extraction. Unsupported Program languages return only graph
  facts.

- [ ] Implement `detect_with_snapshot`. During cold word counting,
  `StatHashIndex::word_counts_with_captures` reads missing files in parallel and
  retains those bytes. Warm detection with valid stat entries performs metadata
  checks and retains no bytes.

- [ ] Keep `detect()` as a compatibility wrapper returning only
  `DetectionSnapshot::detection`.

- [ ] Add digest-addressed cache methods so callers with source bytes do not
  invoke the path-based hashing API and reread files:

```rust
pub fn load_with_hash(
    &self,
    content_hash: &str,
    kind: &CacheKind,
) -> Result<Option<Value>, FileError>;

pub fn save_with_hash(
    &self,
    content_hash: &str,
    value: &Value,
    kind: &CacheKind,
) -> Result<(), FileError>;
```

- [ ] Implement:

```rust
pub(crate) fn extract_deterministic_files(
    root: &Path,
    sources: &[PathBuf],
    captured_source_bytes: &mut BTreeMap<String, Vec<u8>>,
    cache: &Cache,
    force: bool,
    max_workers: Option<usize>,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Result<Vec<DeterministicFileResult>, CoreError>;
```

Each worker consumes captured bytes or reads once, resolves language, computes
digests, loads both caches, calls the combined extractor only when required,
and returns sorted results plus pending cache values.

- [ ] Replace `build_program(root, sources, ...)` with:

```rust
pub(crate) fn build_program_from_evidence(
    root: &Path,
    files: &[DeterministicFileResult],
    options: &BuildOptions,
    cache: &Cache,
) -> Result<ProgramBuild, CoreError>;
```

Use source snapshots for explicit artifact source maps and prepared
`EvidenceBatch` values for syntax evidence. Preserve SCIP limits, artifact
reuse, merge validation, counters, and conflict calculation.

- [ ] Feed AST resolution from the same `DeterministicFileResult` values.
  Preserve ID remapping, semantic retention, resolver behavior, empty-file
  tracking, progress, and cache counters.

- [ ] Pass init's validated `DetectionSnapshot` into:

```rust
pub fn build_graph_with_precomputed_detection(
    options: &BuildOptions,
    detection: DetectionSnapshot,
    operation_started: Instant,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Result<BuildResult, CoreError>;
```

Validate `detection.detection.scan_root` against the canonical build root.
Update/extract call `detect_with_snapshot` normally. Init no longer walks the
repository twice.

- [ ] Add post-change coverage for:

  - combined output equality with standalone graph and Program extractors for
    Python, Rust, TypeScript, TSX, and JavaScript;
  - one parser invocation per supported file using a test-only counter;
  - cold captured bytes and warm stat-only detection;
  - exactly one source read for a 300-file fixture;
  - mixed AST/Program cache hit and miss combinations;
  - init passing validated detection once; and
  - unchanged public graph and Program facts.

- [ ] Verify:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages
cargo test -p compass-files --test contracts
cargo test -p compass-core --test program_pipeline
cargo test -p compass-cli --test init_cli
cargo build --release --locked -p compass-cli --bin compass
benchmark_output=$(mktemp -d /tmp/compass-unified.XXXXXX)
/usr/bin/time -p target/release/compass extract \
  /Users/haipingfu/Github/django \
  --code-only --force --timing --no-cluster --no-viz \
  --out "$benchmark_output"
```

Expected: Django facts match the baseline, and the timing report contains one
deterministic-extraction stage rather than separate Program/AST passes.

- [ ] Commit:

```bash
git add crates/compass-languages/src/combined.rs \
  crates/compass-languages/src/lib.rs \
  crates/compass-languages/src/engine.rs \
  crates/compass-languages/src/program/mod.rs \
  crates/compass-languages/tests/program_evidence.rs \
  crates/compass-languages/tests/engine_edge_coverage.rs \
  crates/compass-core/src/deterministic.rs \
  crates/compass-core/src/lib.rs \
  crates/compass-core/src/pipeline.rs \
  crates/compass-core/src/program.rs \
  crates/compass-files/src/cache.rs \
  crates/compass-files/src/detect.rs \
  crates/compass-files/src/hash.rs \
  crates/compass-files/src/lib.rs \
  crates/compass-files/tests/contracts.rs \
  crates/compass-cli/src/lib.rs \
  crates/compass-cli/src/init_commands.rs \
  crates/compass-cli/tests/init_cli.rs \
  crates/compass-core/tests/program_pipeline.rs
git commit -m "perf(core): build from one parsed source snapshot"
```

### Task 5: Serialize once and overlap independent work

**Files:**

- Modify: `crates/compass-analysis/src/summary.rs`
- Modify: `crates/compass-analysis/tests/summary.rs`
- Modify: `crates/compass-files/src/atomic.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Modify: `crates/compass-files/tests/contracts.rs`
- Create: `crates/compass-core/src/publish.rs`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-core/src/program.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`
- Modify: `crates/compass-core/tests/pipeline_edge_coverage.rs`

**Interfaces:**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrittenArtifact {
    pub bytes: u64,
    pub sha256: String,
}

pub(crate) struct GraphAssembly {
    pub document: GraphDocument,
    pub communities: Communities,
    pub labels: BTreeMap<usize, String>,
    pub analysis: serde_json::Value,
    pub report: Option<String>,
    pub overview: Option<serde_json::Value>,
    pub stats: GraphStats,
}

pub(crate) struct PublishedArtifacts {
    pub graph: ArtifactSeal,
    pub program: Option<ArtifactSeal>,
    pub required: BTreeMap<String, ArtifactSeal>,
}
```

- [ ] Add atomic write helpers that calculate length and SHA-256 while writing.
  For callers that already own canonical bytes, hash that buffer once and write
  it atomically. For streaming serializers, wrap `Write` with a checked counter
  and `Sha256`.

- [ ] Make `ProgramBuild` retain one validated canonical byte vector:

```rust
pub(crate) struct ProgramBuild {
    pub analysis: AnalysisBundle,
    pub canonical_bytes: Vec<u8>,
    pub syntax_analyzed: usize,
    pub syntax_reused: usize,
    pub artifacts_loaded: usize,
    pub artifacts_reused: usize,
    pub artifact_documents_analyzed: usize,
    pub artifact_documents_reused: usize,
    pub conflicts: usize,
}

pub(crate) struct GraphStats {
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
}
```

Create it once after Program analysis and use the same bytes for output and
state seal.

- [ ] Remove production load/save/prune of the complete `ProgramMerge` cache.
  Unchanged builds exit through verified state; changed provider sets merge
  prepared per-file evidence. Keep syntax and artifact caches.

- [ ] Change Program output to return its seal without rereading:

```rust
pub(crate) fn write_program(
    output_dir: &Path,
    canonical_bytes: &[u8],
) -> Result<ArtifactSeal, CoreError>;
```

- [ ] Refactor graph assembly to return `GraphAssembly` without final output
  writes. Preserve semantic shrink guards, community remapping, clustering,
  labels, cohesion, god nodes, surprises, questions, reports, and overview data.

- [ ] Use a scoped thread for Program assembly while graph assembly runs on the
  calling thread. Convert a panic to:

```rust
CoreError::WorkerPanic("program assembly".to_owned())
```

Avoid nested host-sized pools: combined per-file extraction owns the explicit
Rayon pool.

- [ ] Implement `publish_generation` in `publish.rs`. Write graph, Program,
  pending cache records, and independent small artifacts concurrently. Join all
  writers, save the manifest, publish build state last, then commit the guard.

Any writer failure leaves the incomplete marker and does not publish trusted
state.

- [ ] Route clustered, no-cluster, semantic, supplemental, and migration paths
  through the common publication boundary. The verified unchanged path performs
  no writes except explicitly requested removal of optional visualization
  artifacts.

- [ ] Add post-change coverage for:

  - identical one-worker and twelve-worker public artifacts;
  - no `cache/program-merge` output;
  - Program state digest matching public bytes;
  - publication failure leaving incomplete/untrusted state;
  - all clustered/no-cluster and semantic branches; and
  - default visualization behavior on graphs above the HTML node limit.

- [ ] Verify:

```bash
cargo fmt --all -- --check
cargo test -p compass-analysis
cargo test -p compass-files
cargo test -p compass-core
cargo test -p compass-output
cargo test -p compass-graph
cargo build --release --locked -p compass-cli --bin compass
benchmark_output=$(mktemp -d /tmp/compass-concurrent.XXXXXX)
/usr/bin/time -lp target/release/compass extract \
  /Users/haipingfu/Github/django \
  --code-only --force --timing --no-viz \
  --out "$benchmark_output"
```

Record built-in stages, real/user/system time, peak RSS, output counts, and
artifact digests.

- [ ] Commit:

```bash
git add crates/compass-analysis/src/summary.rs \
  crates/compass-analysis/tests/summary.rs \
  crates/compass-files/src/atomic.rs \
  crates/compass-files/src/lib.rs \
  crates/compass-files/tests/contracts.rs \
  crates/compass-core/src/publish.rs \
  crates/compass-core/src/lib.rs \
  crates/compass-core/src/program.rs \
  crates/compass-core/src/pipeline.rs \
  crates/compass-core/src/build_state.rs \
  crates/compass-core/tests/program_pipeline.rs \
  crates/compass-core/tests/pipeline_edge_coverage.rs
git commit -m "perf(core): overlap index assembly and publication"
```

### Task 6: Qualify Django, document behavior, and update the PR

**Files:**

- Create: `scripts/qualify_django_performance.sh`
- Modify: `scripts/test_release_scripts.sh`
- Modify: `docs/implementation/extraction-pipeline.md`
- Modify: `docs/reference/commands.md`
- Modify: `docs/reference/outputs.md`
- Modify: `CHANGELOG.md`

**Qualification output:**

```text
operation sample seconds peak_kib files nodes edges communities modules summaries conflicts graph_sha256 program_sha256
```

- [ ] Implement a guarded script with:

```bash
#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
django_root=${DJANGO_ROOT:-/Users/haipingfu/Github/django}
compass_bin=${COMPASS_BIN:-"$repository_root/target/release/compass"}
samples=${DJANGO_SAMPLES:-3}
limit=${DJANGO_LIMIT_SECONDS:-5.0}
qualification_root=$(mktemp -d)
django_worktree="$qualification_root/django"
results=${DJANGO_BENCH_OUTPUT:-"$repository_root/target/django-performance.tsv"}
qualification_summary=${DJANGO_BENCH_SUMMARY:-"$repository_root/target/django-performance-summary.md"}
```

Resolve Django through `git rev-parse --show-toplevel`. Reject empty, `/`, the
home directory, and non-Git paths. Capture the source status, create a detached
worktree under the temporary root, and verify source status is unchanged at the
end.

- [ ] Measure each command with `scripts/measure_process.py`:

```bash
COMPASS_OUT=compass-perf-out "$compass_bin" init . --yes --force --timing
COMPASS_OUT=compass-perf-out "$compass_bin" update . --force --timing
COMPASS_OUT=compass-perf-out "$compass_bin" update . --timing
```

For every cold-init sample, remove only the validated temporary worktree's
`compass-perf-out` and `.compass/config.toml`. Never remove or modify an output
in the user's Django checkout.

- [ ] Record built-in total, independent monotonic wall time, peak RSS, Django
  and Compass commits, machine CPU count, output counts, and SHA-256 of public
  artifacts.

- [ ] Use an embedded Python summary to print every sample, median, and maximum.
  Fail when:

  - any duration is greater than or equal to 5.0s;
  - counts differ between equivalent full builds;
  - canonical public artifact digests differ unexpectedly; or
  - the user's Django repository status changes.

- [ ] Extend release-script coverage with a tiny Git fixture and stub Compass.
  Verify unsafe-root rejection, `DJANGO_SAMPLES=1`, exact cleanup boundaries,
  unchanged source status, and rejection of a `5.000000000` sample.

- [ ] Run the full gate:

```bash
cargo build --release --locked -p compass-cli --bin compass
DJANGO_ROOT=/Users/haipingfu/Github/django \
  COMPASS_BIN=target/release/compass \
  DJANGO_SAMPLES=3 \
  bash scripts/qualify_django_performance.sh
```

If a stage is still above its budget, optimize only the measured stage while
preserving output counts and digests, then rerun all three workflows. The goal
is not complete until every sample is below 5.0s.

- [ ] Document:

  - default elapsed lines;
  - `--timing` for init/extract/update;
  - overlapping stage interpretation;
  - `.compass_build_state.json` as an internal integrity seal;
  - MessagePack caches as disposable internal data;
  - unchanged public JSON formats;
  - the one-read/one-parse pipeline; and
  - the exact Django qualification command.

- [ ] Run final verification:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
bash scripts/test_release_scripts.sh
DJANGO_ROOT=/Users/haipingfu/Github/django \
  COMPASS_BIN=target/release/compass \
  DJANGO_SAMPLES=3 \
  bash scripts/qualify_django_performance.sh
git diff --check
git status --short
```

- [ ] Commit qualification and docs:

```bash
git add scripts/qualify_django_performance.sh \
  scripts/test_release_scripts.sh \
  docs/implementation/extraction-pipeline.md \
  docs/reference/commands.md \
  docs/reference/outputs.md \
  CHANGELOG.md
git commit -m "test(perf): gate Django indexing below five seconds"
```

- [ ] Push and update draft PR #48:

```bash
git push origin codex/fix-python-symbol-collisions
gh pr edit 48 \
  --title "Fix repeated Python symbols and accelerate Django indexing"
gh pr comment 48 --body-file "$qualification_summary"
```

The PR evidence must include all nine samples, medians, maxima, exact commits,
CPU count, counts, digests, built-in stage output, and verification commands.
Keep the PR in draft while any acceptance item is missing.

---

## Completion audit

Before declaring completion, verify each requirement against authoritative
evidence:

| Requirement | Evidence |
| --- | --- |
| Cold init below 5s | Three release samples, each `<5.0`, from guarded script |
| Forced full build below 5s | Three release samples, each `<5.0` |
| Unchanged update below 5s | Three release samples, each `<5.0` |
| Built-in duration | Captured stdout for init, extract, update |
| Detailed stages | Captured `--timing` stderr for all operations |
| Output preservation | Counts and canonical public artifact digests |
| One source read | Instrumented 300-file regression coverage |
| One supported parse | Combined extractor counter/equivalence coverage |
| Safe fast path | Corruption, mismatch, and interrupted-state coverage |
| Incremental reuse | Edited-source core pipeline coverage |
| Repository quality | fmt, clippy, workspace tests, release-script tests |
| PR delivery | Pushed commits and PR #48 qualification evidence |

Do not substitute a median-only pass, a warmed cold build, a no-cluster build,
or a narrower fixture for the complete Django acceptance commands.
