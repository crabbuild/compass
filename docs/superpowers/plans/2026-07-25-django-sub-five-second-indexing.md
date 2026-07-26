# Django Sub-Five-Second Indexing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make cold Django initialization, forced full indexing, and unchanged updates each complete in less than five seconds while preserving deterministic graph and Program IR outputs and reporting built-in wall time.

**Architecture:** Replace the duplicate source-read/tree-sitter paths with one combined deterministic extraction pass, use versioned MessagePack for internal caches, and assemble graph and Program outputs concurrently. Seal each completed output generation with artifact digests and saved statistics so unchanged updates validate bytes without parsing hundreds of megabytes of JSON.

**Tech Stack:** Rust 2024, tree-sitter, Rayon, serde, serde_json, rmp-serde, sha2, Bash, Python 3 benchmark helper

## Global Constraints

- All three cold init samples must finish in less than 5.0 seconds.
- All three forced full update samples must finish in less than 5.0 seconds.
- All three unchanged update samples must finish in less than 5.0 seconds.
- Public `graph.json` and `program.json` remain deterministic JSON.
- Correct graph, Program IR, clustering, reports, and caches must be complete before the command exits.
- Production indexing may not add Python, network, daemon, or Django-specific behavior.
- Explicit `--force` never takes an unchanged fast path.
- Missing, stale, truncated, or modified artifacts fail safe to validation or rebuilding.
- Use release binaries for performance evidence.
- Run `graphify update .` after code changes.

---

## File structure

The implementation uses these responsibility boundaries:

- `crates/compass-languages/src/combined.rs`: one-parse graph and Program extraction API.
- `crates/compass-files/src/cache.rs`: versioned MessagePack cache transport with JSON migration reads.
- `crates/compass-files/src/atomic.rs`: atomic serialization that returns byte length and SHA-256.
- `crates/compass-core/src/build_state.rs`: sealed generation schema, capture, and verification.
- `crates/compass-core/src/deterministic.rs`: one-read source snapshots and cache-aware combined extraction.
- `crates/compass-core/src/program.rs`: Program assembly from prepared evidence; no source reread or complete merge cache.
- `crates/compass-core/src/pipeline.rs`: orchestration, graph/Program concurrency, publication, and timings.
- `crates/compass-cli/src/lib.rs`: update/extract timing parsing and output.
- `crates/compass-cli/src/init_commands.rs`: init timing and reuse of validated detection.
- `scripts/qualify_django_performance.sh`: guarded end-to-end acceptance benchmark.

Files that already own format-specific extraction, graph building, clustering,
and reporting remain responsible for those behaviors.

### Task 1: Make operation timing a tested CLI contract

**Files:**

- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/init_commands.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/tests/init_cli.rs`
- Modify: `crates/compass-cli/tests/program_cli.rs`

**Interfaces:**

- Produces: `BuildOperation::{Init, Extract, Update}`
- Produces: `format_operation_elapsed(operation: BuildOperation, elapsed: Duration) -> String`
- Produces: `BuildTimings { detect, deterministic_extract, graph_assembly, program_analysis, publish }`
- Produces: `--timing` support for init, extract, and update
- Consumed by: Tasks 5–8 for stable stage reporting and benchmark parsing

- [ ] **Step 1: Add failing CLI timing assertions**

Add a helper and assertions to the integration tests:

```rust
fn assert_elapsed_line(output: &[u8], operation: &str) {
    let text = String::from_utf8_lossy(output);
    let prefix = format!("Compass {operation} completed in ");
    let line = text
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("missing elapsed line in:\n{text}"));
    assert!(line.ends_with("s."));
    let seconds = line[prefix.len()..line.len() - 2]
        .parse::<f64>()
        .unwrap_or_else(|error| panic!("invalid elapsed value: {error}"));
    assert!(seconds >= 0.0);
}
```

In `init_cli.rs`, assert `init` output contains the elapsed line. In
`program_cli.rs` or the existing build CLI coverage test, run both
`extract --code-only --timing` and `update --timing` and assert:

```rust
assert_elapsed_line(&output.stdout, "update");
assert!(String::from_utf8_lossy(&output.stderr)
    .contains("[compass timing] deterministic extract:"));
assert!(String::from_utf8_lossy(&output.stderr)
    .contains("[compass timing] total:"));
```

- [ ] **Step 2: Run the focused tests and confirm the contract fails**

Run:

```bash
cargo test -p compass-cli --test init_cli
cargo test -p compass-cli --test program_cli
```

Expected: the new assertions fail because init/update do not print a total and
update rejects `--timing`.

- [ ] **Step 3: Introduce the operation and stable timing model**

In `crates/compass-core/src/pipeline.rs`, replace the old timing fields with:

```rust
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildTimings {
    pub detect: Duration,
    pub deterministic_extract: Duration,
    pub graph_assembly: Duration,
    pub program_analysis: Duration,
    pub publish: Duration,
}
```

In `crates/compass-cli/src/lib.rs`, add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildOperation {
    Init,
    Extract,
    Update,
}

impl BuildOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Extract => "extract",
            Self::Update => "update",
        }
    }
}

fn format_operation_elapsed(operation: BuildOperation, elapsed: Duration) -> String {
    format!(
        "Compass {} completed in {:.2}s.",
        operation.label(),
        elapsed.as_secs_f64()
    )
}
```

Change `command_build_with_validation` to receive `BuildOperation`. Start its
timer exactly once and append the elapsed line to successful stdout. Append
`Compass <operation> failed after X.XXs.` to stderr on a build failure.

- [ ] **Step 4: Accept `--timing` for update and init**

In the build parser, change:

```rust
"--timing" if extract => timing = true,
```

to:

```rust
"--timing" => timing = true,
```

Add `timing: bool` to `InitOptions`, parse `--timing`, and append it to the
internal build arguments. Route init through `BuildOperation::Init`; direct
extract and update use their matching variants.

Update help text to show:

```text
compass init [PATH] [--yes] [--force] [--timing]
compass update [PATH] [--force] [--timing]
```

- [ ] **Step 5: Replace the old stage formatter**

Make `format_extract_timings` operation-independent:

```rust
fn format_build_timings(elapsed: Duration, timings: &BuildTimings) -> String {
    [
        ("detect", timings.detect),
        ("deterministic extract", timings.deterministic_extract),
        ("graph assembly", timings.graph_assembly),
        ("program analysis", timings.program_analysis),
        ("publish", timings.publish),
    ]
    .into_iter()
    .map(|(name, value)| {
        format!("[compass timing] {name}: {:.1}s", value.as_secs_f64())
    })
    .chain(std::iter::once(format!(
        "[compass timing] total: {:.1}s",
        elapsed.as_secs_f64()
    )))
    .collect::<Vec<_>>()
    .join("\n")
}
```

Concurrent stages introduced later may overlap; do not sum them to compute the
total.

- [ ] **Step 6: Run focused tests and formatting**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-cli --test init_cli
cargo test -p compass-cli --test program_cli
cargo test -p compass-cli --lib
```

Expected: all pass and each operation emits exactly one elapsed line.

- [ ] **Step 7: Commit the timing contract**

```bash
git add crates/compass-core/src/pipeline.rs \
  crates/compass-cli/src/lib.rs \
  crates/compass-cli/src/init_commands.rs \
  crates/compass-cli/src/help.rs \
  crates/compass-cli/tests/init_cli.rs \
  crates/compass-cli/tests/program_cli.rs
git commit -m "feat(cli): report index wall time"
```

### Task 2: Add a fail-safe sealed generation fast path

**Files:**

- Create: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`
- Modify: `crates/compass-core/tests/pipeline_edge_coverage.rs`

**Interfaces:**

- Produces: `BUILD_STATE_FILE: &str = ".compass_build_state.json"`
- Produces: `BuildProfile::from_options(options: &BuildOptions) -> BuildProfile`
- Produces: `ArtifactSeal::capture(path: &Path) -> Result<ArtifactSeal, CoreError>`
- Produces: `BuildState::verify(...) -> Result<Option<VerifiedBuildState>, CoreError>`
- Produces: saved graph and Program statistics without JSON deserialization
- Consumed by: Tasks 6–8

- [ ] **Step 1: Add fast-path and corruption tests**

Extend `program_pipeline_is_deterministic_incremental_and_uses_program_json`
with:

```rust
let state = cold.output_dir.join(".compass_build_state.json");
assert!(state.is_file());

let warm = build_local_graph(&options)?;
assert_eq!(warm.files_extracted, 0);
assert_eq!(warm.program_syntax_reused, 1);
assert!(warm.timings.program_analysis.is_zero());

let mut corrupted = fs::read(&output)?;
corrupted[corrupted.len() / 2] ^= 1;
fs::write(&output, corrupted)?;
let repaired = build_local_graph(&options)?;
assert_eq!(repaired.program_syntax_reused, 1);
assert_eq!(fs::read(&output)?, cold_bytes);
```

Add cases for:

```rust
fs::remove_file(&state)?;
fs::write(&output, &cold_bytes[..cold_bytes.len() / 2])?;
fs::write(&state, b"{\"schema\":999}")?;
fs::write(cold.output_dir.join(".compass-build-incomplete"), b"1")?;
```

Each case must rebuild or validate safely and restore a readable Program
artifact. Add a profile mismatch test by changing `options.no_cluster`.

- [ ] **Step 2: Run the core tests and confirm failure**

Run:

```bash
cargo test -p compass-core --test program_pipeline
cargo test -p compass-core --test pipeline_edge_coverage
```

Expected: failures because the state file and verified-statistics path do not
exist.

- [ ] **Step 3: Define the build-state schema**

Create `build_state.rs` with:

```rust
pub(crate) const BUILD_STATE_FILE: &str = ".compass_build_state.json";
const BUILD_STATE_SCHEMA: &str = "compass.build-state/1";

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BuildState {
    pub schema: String,
    pub profile: BuildProfile,
    pub manifest: ArtifactSeal,
    pub graph: ArtifactSeal,
    pub program: Option<ArtifactSeal>,
    pub required: BTreeMap<String, ArtifactSeal>,
    pub stats: SavedStats,
}
```

`ArtifactSeal::capture` streams a file through a 1 MB buffer into `Sha256` and
records metadata length. It rejects non-regular files.

- [ ] **Step 4: Implement strict state verification**

Implement:

```rust
pub(crate) fn load_verified(
    output_dir: &Path,
    profile: &BuildProfile,
    manifest_path: &Path,
    prior_build_complete: bool,
) -> Result<Option<BuildState>, CoreError>
```

Return `Ok(None)` for absent, malformed, wrong-schema, profile-mismatched, or
seal-mismatched state. Propagate only I/O errors that prevent a safe rebuild.
Verify the manifest, graph, optional Program, and every required artifact seal.
Do not deserialize `graph.json` or `program.json`.

- [ ] **Step 5: Integrate verification before the unchanged return**

Before `BuildGuard::begin`, record:

```rust
let prior_build_complete = BuildGuard::ensure_complete(&output_dir).is_ok();
```

Begin the new guard, run detection and the manifest check, then call
`load_verified`. Replace `load_current_program` and `unchanged_output_stats` in
the unchanged return with the verified state's statistics.

Set:

```rust
program_syntax_reused = state.stats.program_modules;
program_artifacts_reused =
    state.stats.program_providers.saturating_sub(state.stats.program_modules);
```

Keep the old validation path only as migration fallback when no state exists.

- [ ] **Step 6: Publish state last**

After every successful output path has written the manifest, graph, Program,
and required side artifacts, capture their seals and write the state atomically.
Then call `guard.commit()`.

Never save state in an error path. Remove the stale state before a forced build
starts publishing so a crash cannot leave it trusted against partially replaced
artifacts.

- [ ] **Step 7: Run core tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-core --test program_pipeline
cargo test -p compass-core --test pipeline_edge_coverage
cargo test -p compass-core --lib
```

Expected: valid state uses the no-deserialization path; every corruption case
falls back and repairs output.

- [ ] **Step 8: Benchmark the unchanged update**

Build and run:

```bash
cargo build --release --locked -p compass-cli --bin compass
/usr/bin/time -p target/release/compass update /Users/haipingfu/Github/django --no-viz
```

Expected: successful update below 5.0 seconds with a built-in elapsed line.
Record the exact result in the implementation notes.

- [ ] **Step 9: Commit sealed state**

```bash
git add crates/compass-core/src/build_state.rs \
  crates/compass-core/src/lib.rs \
  crates/compass-core/src/pipeline.rs \
  crates/compass-core/tests/program_pipeline.rs \
  crates/compass-core/tests/pipeline_edge_coverage.rs
git commit -m "perf(core): seal completed index generations"
```

### Task 3: Replace deterministic JSON caches with MessagePack

**Files:**

- Modify: `crates/compass-files/Cargo.toml`
- Modify: `crates/compass-files/src/cache.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Modify: `crates/compass-files/tests/contracts.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`

**Interfaces:**

- Produces: `CACHE_ENCODING_VERSION: u32 = 1`
- Produces: `.msgpack` AST and Program cache entries
- Produces: read-only `.json` migration fallback
- Preserves: `Cache::load`, `save`, `save_batch`, `load_program`, and
  `save_program` caller signatures
- Consumed by: Task 5's combined cache-aware extraction

- [ ] **Step 1: Add failing MessagePack cache tests**

In `contracts.rs`, add:

```rust
#[test]
fn deterministic_caches_write_msgpack_and_read_legacy_json()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("main.rs");
    fs::write(&source, "fn main() {}\n")?;
    let mut cache = Cache::new(directory.path(), None)?;
    let value = json!({"nodes": [], "edges": [], "hyperedges": []});

    cache.save(&source, &value, &CacheKind::Ast, None)?;
    let ast_dir = cache.directory(&CacheKind::Ast, None);
    assert_eq!(
        fs::read_dir(&ast_dir)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "msgpack"))
            .count(),
        1
    );
    assert_eq!(
        cache.load(&source, &CacheKind::Ast, None, false, false)?,
        Some(value)
    );
    Ok(())
}
```

Add Program cache round-trip, malformed MessagePack-as-cache-miss, pruning of
both extensions, and JSON migration cases.

- [ ] **Step 2: Run the cache tests and confirm failure**

Run:

```bash
cargo test -p compass-files --test contracts deterministic_caches
cargo test -p compass-files --test contracts program_cache
```

Expected: the extension and migration assertions fail.

- [ ] **Step 3: Add the existing workspace dependency**

Add:

```toml
rmp-serde.workspace = true
```

to `crates/compass-files/Cargo.toml`. Do not add another serialization crate.

- [ ] **Step 4: Add versioned binary helpers**

In `cache.rs`, define:

```rust
const CACHE_ENCODING_VERSION: u32 = 1;
const MESSAGEPACK_EXTENSION: &str = "msgpack";

fn encode_cache<T: Serialize>(value: &T) -> Result<Vec<u8>, FileError> {
    rmp_serde::to_vec_named(value).map_err(|error| {
        FileError::CacheEncoding(error.to_string())
    })
}

fn decode_cache<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    rmp_serde::from_slice(bytes).ok()
}
```

Add `FileError::CacheEncoding(String)`. Include `e1` in deterministic cache
directories so older encodings cannot collide.

- [ ] **Step 5: Convert AST cache reads and writes**

Write `.msgpack` atomically from `save` and `save_batch`. `load` tries the
MessagePack path first, then the legacy `.json` path. Both decoded `Value`
representations still pass through `absolutize_source_files`.

Keep concurrent batch publication:

```rust
jobs.into_par_iter().try_for_each(|(destination, value)| {
    let mut on_disk = value.clone();
    relativize_source_files(&mut on_disk, root);
    let bytes = encode_cache(&on_disk)?;
    write_bytes_atomic(destination, &bytes)
})
```

- [ ] **Step 6: Convert Program cache reads and writes**

Use the same extension order for `load_program` and `save_program`. Program
values remain repository-relative. Update pruning and `cached_files` to accept
both `.msgpack` and `.json`, preferring MessagePack when both exist.

- [ ] **Step 7: Run cache and Program pipeline tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-files --test contracts
cargo test -p compass-core --test program_pipeline
```

Expected: cache corruption is a miss, migration reads work, and new entries are
MessagePack.

- [ ] **Step 8: Measure cold cache size and time**

Run one cold no-cluster extraction into a fresh `mktemp -d` output:

```bash
benchmark_output=$(mktemp -d /tmp/compass-msgpack.XXXXXX)
/usr/bin/time -p target/release/compass extract \
  /Users/haipingfu/Github/django \
  --code-only --force --timing --no-cluster --no-viz \
  --out "$benchmark_output"
du -sh "$benchmark_output/compass-out/cache"
```

Record wall time and cache size. Preserve the output until the next task's
comparison is captured.

- [ ] **Step 9: Commit the cache format**

```bash
git add crates/compass-files/Cargo.toml \
  crates/compass-files/src/cache.rs \
  crates/compass-files/src/lib.rs \
  crates/compass-files/tests/contracts.rs \
  crates/compass-core/tests/program_pipeline.rs
git commit -m "perf(cache): encode deterministic entries with MessagePack"
```

### Task 4: Add one-parse graph and Program extraction

**Files:**

- Create: `crates/compass-languages/src/combined.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/src/program/mod.rs`
- Modify: `crates/compass-languages/tests/program_evidence.rs`
- Modify: `crates/compass-languages/tests/engine_edge_coverage.rs`

**Interfaces:**

- Produces: `CombinedExtraction { graph: Extraction, program: Option<EvidenceBatch> }`
- Produces: `Engine::extract_source_combined(path, source_file, source)`
- Produces: `program::extract_from_tree(...)`
- Consumed by: Task 5

- [ ] **Step 1: Add combined-equivalence tests**

For Python, Rust, TypeScript, TSX, and JavaScript fixtures:

```rust
let mut combined_engine = Engine::default();
let combined = combined_engine.extract_source_combined(
    Path::new("/repo/src/sample.py"),
    "src/sample.py",
    source,
)?;

let mut graph_engine = Engine::default();
let expected_graph =
    graph_engine.extract_source(Path::new("/repo/src/sample.py"), source)?;
let mut provider = TreeSitterSyntaxProvider::default();
let expected_program = provider
    .analyze_file(FileInput {
        source_file: "src/sample.py",
        language: "python",
        source,
    })?
    .ok_or("missing Program evidence")?;

assert_eq!(combined.graph, expected_graph);
assert_eq!(combined.program, Some(expected_program));
```

Add one unsupported Program language fixture asserting `program.is_none()` and
the graph remains identical.

- [ ] **Step 2: Add an internal one-parse test**

Under `#[cfg(test)]`, add `parse_invocations: usize` to `Engine`, increment it
inside `parse`, and test:

```rust
let mut engine = Engine::default();
let result = engine.extract_source_combined(
    Path::new("sample.py"),
    "sample.py",
    b"def run():\n    return 1\n",
)?;
assert!(result.program.is_some());
assert_eq!(engine.parse_invocations, 1);
```

The field and assertion helper remain test-only.

- [ ] **Step 3: Run tests and confirm failure**

Run:

```bash
cargo test -p compass-languages combined
cargo test -p compass-languages one_parse
```

Expected: compilation fails because the combined API does not exist.

- [ ] **Step 4: Extract the shared Program tree function**

In `program/mod.rs`, move descriptor construction and the language match into:

```rust
pub(crate) fn extract_from_tree(
    source_file: &str,
    language: &'static str,
    source: &[u8],
    root: tree_sitter::Node<'_>,
) -> Result<EvidenceBatch, ProviderError>
```

Normalize `source_file`, construct the same `ProviderDescriptor`, dispatch to
the existing Python/Rust/TypeScript extractors, and validate with
`merge_evidence(vec![batch.clone()])`. Make
`TreeSitterSyntaxProvider::analyze_file` call this function after its parse.

- [ ] **Step 5: Split generic graph extraction at the parsed-tree boundary**

In `engine.rs`, extract:

```rust
fn extract_generic_tree(
    path: &Path,
    spec: LanguageSpec,
    source: &[u8],
    tree: &Tree,
) -> Extraction
```

Move the current language dispatch, Python rationale, and definition metadata
into it. `extract_generic_source` still handles source-driven languages and
Objective-C masking before calling this helper.

- [ ] **Step 6: Implement the combined API**

Create:

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct CombinedExtraction {
    pub graph: Extraction,
    pub program: Option<EvidenceBatch>,
}
```

For Program-supported generic languages, parse once, send the root to
`extract_generic_tree` and `extract_from_tree`, and return both values. For all
other language kinds, call `extract_source` and return `program: None`.

Map Program provider failures to:

```rust
ExtractError::InvalidProgramEvidence {
    path: path.to_path_buf(),
    detail: error.to_string(),
}
```

- [ ] **Step 7: Run language verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages --test program_evidence
cargo test -p compass-languages --test engine_edge_coverage
cargo test -p compass-languages --lib
```

Expected: every combined result equals the two prior standalone results and the
one-parse test reports one parser invocation.

- [ ] **Step 8: Commit combined extraction**

```bash
git add crates/compass-languages/src/combined.rs \
  crates/compass-languages/src/lib.rs \
  crates/compass-languages/src/engine.rs \
  crates/compass-languages/src/program/mod.rs \
  crates/compass-languages/tests/program_evidence.rs \
  crates/compass-languages/tests/engine_edge_coverage.rs
git commit -m "perf(languages): share graph and Program parses"
```

### Task 5: Build from one source snapshot and reuse init detection

**Files:**

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

**Interfaces:**

- Produces: `SourceSnapshot`
- Produces: `DeterministicFileResult`
- Produces: `extract_deterministic_files(...)`
- Produces: `DetectionSnapshot` and `detect_with_snapshot(...)`
- Produces: `build_program_from_evidence(...)`
- Produces: `build_graph_with_precomputed_detection(...)`
- Consumed by: Tasks 6–8

- [ ] **Step 1: Add source-read and init-detection regression tests**

Add a `compass-files` test proving cold detection captures bytes once and warm
detection does not reread unchanged files:

```rust
let cold = detect_with_snapshot(root, &DetectOptions::default())?;
assert_eq!(
    cold.source_bytes.get(&canonical_source).map(Vec::as_slice),
    Some(b"def run():\n    return 1\n".as_slice())
);
let warm = detect_with_snapshot(root, &DetectOptions::default())?;
assert!(!warm.source_bytes.contains_key(&canonical_source));
assert_eq!(warm.detection.total_words, cold.detection.total_words);
```

Add a core test-only observer:

```rust
#[derive(Default)]
struct DeterministicObserver {
    source_reads: AtomicUsize,
    combined_parses: AtomicUsize,
}
```

Build a 300-file Python fixture with Program analysis enabled and assert:

```rust
assert_eq!(observer.source_reads.load(Ordering::Relaxed), 300);
assert_eq!(observer.combined_parses.load(Ordering::Relaxed), 300);
```

In `init_cli.rs`, make the injected builder receive the `DetectionSnapshot` that init
already produced and assert its `scan_root` and file count. This test fails if
the builder performs an independent detect.

- [ ] **Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p compass-core --test program_pipeline one_source_snapshot
cargo test -p compass-cli --test init_cli reuses_validated_detection
```

Expected: compilation or assertions fail because there is no detection/source
snapshot API and
init does not pass detection to the build.

- [ ] **Step 3: Return cold word-count bytes from detection**

In `detect.rs`, add:

```rust
#[derive(Debug, Clone)]
pub struct DetectionSnapshot {
    pub detection: Detection,
    pub source_bytes: BTreeMap<String, Vec<u8>>,
}

pub fn detect_with_snapshot(
    root: &Path,
    options: &DetectOptions,
) -> Result<DetectionSnapshot, FileError>
```

Refactor `detect` to call `detect_with_snapshot` and return only `.detection`.
Add `StatHashIndex::word_counts_with_captures` in `hash.rs`. It resolves
metadata and cached counts in input order, computes missing counts and reads in
parallel with Rayon, updates the index sequentially, and returns
`(count, Option<Vec<u8>>)` for every input path. Bytes are retained only when a
cold count actually read that file.

Use the existing PDF, DOCX, and XLSX exclusions. Key `source_bytes` by canonical
path string. A warm detection with valid stat entries performs metadata checks
but captures no bytes.

- [ ] **Step 4: Add digest-addressed cache operations**

In `Cache`, add:

```rust
pub fn load_with_hash(
    &self,
    content_hash: &str,
    kind: &CacheKind,
) -> Result<Option<Value>, FileError>

pub fn save_with_hash(
    &self,
    content_hash: &str,
    value: &Value,
    kind: &CacheKind,
) -> Result<(), FileError>
```

Add matching generic Program methods keyed by the existing logical key. The
snapshot path supplies hashes so these operations never reread a source file.
Keep the current path-based methods as compatibility wrappers.

- [ ] **Step 5: Create the source snapshot model**

In `deterministic.rs`, define:

```rust
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

Create snapshots in the build-local Rayon pool. Canonicalize each path, verify
it remains below the repository root, consume captured detection bytes when
present, otherwise read bytes once, resolve language once, and compute both
required digests from those bytes.

- [ ] **Step 6: Make extraction cache-aware and combined**

Implement:

```rust
pub(crate) fn extract_deterministic_files(
    root: &Path,
    sources: &[PathBuf],
    captured_source_bytes: &mut BTreeMap<String, Vec<u8>>,
    cache: &Cache,
    force: bool,
    max_workers: Option<usize>,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Result<Vec<DeterministicFileResult>, CoreError>
```

For each snapshot:

1. load AST and Program syntax caches by digest unless forced;
2. if either required value is missing, call
   `Engine::extract_source_combined` once;
3. keep a valid cached side and take only the missing side from the combined
   result;
4. retain newly produced records in `pending_graph_cache` and
   `pending_program_cache` for atomic publication after graph and Program
   assembly; and
5. return results sorted by `source_file`.

Do not clone source bytes into a second collection.

- [ ] **Step 7: Assemble Program IR from prepared evidence**

Replace `build_program(root, sources, ...)` with:

```rust
pub(crate) fn build_program_from_evidence(
    root: &Path,
    files: &[DeterministicFileResult],
    options: &BuildOptions,
    cache: &Cache,
) -> Result<ProgramBuild, CoreError>
```

Use snapshots for source digests/text required by explicit artifacts. Use
prepared `EvidenceBatch` values for syntax providers. Preserve artifact
discovery, SCIP limits, merge validation, counters, and conflict calculation.
Delete the old sequential `read_sources` call from the cold path.

- [ ] **Step 8: Feed graph extraction from the same results**

In `pipeline.rs`, remove the separate `Engine::extract_source` loop. Populate
the AST extraction map and source text from `DeterministicFileResult`. Preserve
AST ID remapping, semantic-layer retention, resolver behavior, empty-file
tracking, cache flush, and progress events.

Set `timings.deterministic_extract` around snapshot plus combined extraction.

- [ ] **Step 9: Pass init's validated detection into core**

Add:

```rust
pub fn build_graph_with_precomputed_detection(
    options: &BuildOptions,
    detection: DetectionSnapshot,
    operation_started: Instant,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Result<BuildResult, CoreError>
```

Validate that `detection.detection.scan_root` canonicalizes to `options.root`.
The init builder closure receives and moves this `DetectionSnapshot` into
`command_build_with_precomputed_detection`; update and extract continue to
call `detect_with_snapshot` normally.

Start init's performance timer immediately before the one validation detection
so its printed total covers actual non-interactive indexing setup without
including a human prompt wait.

- [ ] **Step 10: Run focused verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages
cargo test -p compass-files --test contracts
cargo test -p compass-core --test program_pipeline
cargo test -p compass-cli --test init_cli
```

Expected: combined outputs remain equal, source reads equal source count, and
init performs one detection.

- [ ] **Step 11: Benchmark the cold no-cluster path**

Run into a fresh output:

```bash
benchmark_output=$(mktemp -d /tmp/compass-unified.XXXXXX)
/usr/bin/time -p target/release/compass extract \
  /Users/haipingfu/Github/django \
  --code-only --force --timing --no-cluster --no-viz \
  --out "$benchmark_output"
```

Compare file/node/edge/Program counts to the baseline and record the stage
change. Do not accept a faster result with changed facts.

- [ ] **Step 12: Commit the one-snapshot pipeline**

```bash
git add crates/compass-core/src/deterministic.rs \
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
git commit -m "perf(core): build from one source snapshot"
```

### Task 6: Serialize Program IR once and remove the duplicate merge cache

**Files:**

- Modify: `crates/compass-analysis/src/summary.rs`
- Modify: `crates/compass-analysis/tests/summary.rs`
- Modify: `crates/compass-files/src/atomic.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Modify: `crates/compass-files/tests/contracts.rs`
- Modify: `crates/compass-core/src/program.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`

**Interfaces:**

- Produces: `WrittenArtifact { bytes: u64, sha256: String }`
- Produces: `write_bytes_hashed_atomic(...) -> WrittenArtifact`
- Produces: `AnalysisBundle::validated_canonical_bytes()`
- Produces: `write_program(...) -> ArtifactSeal`
- Removes: production use of `CacheKind::ProgramMerge`
- Consumed by: Task 7 publication

- [ ] **Step 1: Add single-serialization tests**

Add an atomic writer test:

```rust
let artifact = write_bytes_hashed_atomic(&path, b"canonical\n")?;
assert_eq!(artifact.bytes, 10);
assert_eq!(
    artifact.sha256,
    compass_ir::hex_sha256(b"canonical\n")
);
```

In Program pipeline tests, assert:

```rust
assert!(!cold.output_dir.join("cache/program-merge").exists());
let state: serde_json::Value =
    serde_json::from_slice(&fs::read(cold.output_dir.join(".compass_build_state.json"))?)?;
assert_eq!(
    state["program"]["sha256"],
    compass_ir::hex_sha256(&fs::read(&output)?)
);
```

- [ ] **Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p compass-files hashed_atomic
cargo test -p compass-core --test program_pipeline program_pipeline_is_deterministic
```

Expected: hashed writer is missing and the merge cache still exists.

- [ ] **Step 3: Add hashed atomic writes**

In `atomic.rs`, define:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrittenArtifact {
    pub bytes: u64,
    pub sha256: String,
}

pub fn write_bytes_hashed_atomic(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<WrittenArtifact, FileError> {
    let artifact = WrittenArtifact {
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        sha256: hex_sha256(bytes),
    };
    write_bytes_atomic(path, bytes)?;
    Ok(artifact)
}
```

Use a hashing `Write` adapter for graph/output serializers that do not already
own bytes. The adapter updates `Sha256` and a checked byte counter on every
successful write.

- [ ] **Step 4: Avoid repeated Program validation**

Add:

```rust
pub fn validated_canonical_bytes(&self) -> Result<Vec<u8>, AnalysisError> {
    self.validate()?;
    let canonical = self.canonicalized();
    Ok(canonical_json_bytes(&canonical)?)
}
```

Make `canonical_bytes()` delegate to this method so offline behavior is
unchanged. In the core build path, retain one validated canonical byte vector
inside `ProgramBuild`:

```rust
pub(crate) struct ProgramBuild {
    pub analysis: AnalysisBundle,
    pub canonical_bytes: Vec<u8>,
    // existing counters
}
```

Construct it once after Program analysis.

- [ ] **Step 5: Remove the complete Program merge cache**

Delete the `ProgramMerge` load/save/prune block from `build_program_from_evidence`.
An unchanged provider set exits through sealed state; a changed set merges the
prepared per-file batches. Keep per-file syntax and artifact caches.

Leave the `CacheKind::ProgramMerge` enum variant for one release only if another
public caller still constructs it; otherwise remove it and update exhaustive
matches.

- [ ] **Step 6: Write Program output and seal from the same bytes**

Change:

```rust
pub(crate) fn write_program(
    output_dir: &Path,
    canonical_bytes: &[u8],
) -> Result<ArtifactSeal, CoreError>
```

Call `write_bytes_hashed_atomic`, convert `WrittenArtifact` into `ArtifactSeal`,
and pass the seal directly to build-state publication. Do not reread
`program.json` to seal it.

- [ ] **Step 7: Run focused tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-analysis
cargo test -p compass-files
cargo test -p compass-core --test program_pipeline
```

Expected: public Program bytes remain identical to the pre-change fixture,
state digest matches, and no complete merge cache is produced.

- [ ] **Step 8: Re-run the Django cold profile**

Run the Task 5 command into another fresh output. Record:

- deterministic extraction duration;
- Program analysis duration;
- publish duration;
- total cache size; and
- total wall time.

Expected: 272 MB duplicate cache and one full Program serialization are gone.

- [ ] **Step 9: Commit single Program publication**

```bash
git add crates/compass-analysis/src/summary.rs \
  crates/compass-analysis/tests/summary.rs \
  crates/compass-files/src/atomic.rs \
  crates/compass-files/src/lib.rs \
  crates/compass-files/tests/contracts.rs \
  crates/compass-core/src/program.rs \
  crates/compass-core/src/pipeline.rs \
  crates/compass-core/src/build_state.rs \
  crates/compass-core/tests/program_pipeline.rs
git commit -m "perf(program): serialize analyzed output once"
```

### Task 7: Run independent assembly and publication work concurrently

**Files:**

- Create: `crates/compass-core/src/publish.rs`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/program.rs`
- Modify: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-core/tests/program_pipeline.rs`
- Modify: `crates/compass-core/tests/pipeline_edge_coverage.rs`

**Interfaces:**

- Produces: `GraphAssembly`
- Produces: `PublishedArtifacts`
- Produces: `publish_generation(...)`
- Preserves: deterministic ordering and BuildGuard state
- Consumed by: Task 8 acceptance qualification

- [ ] **Step 1: Add deterministic concurrency tests**

Build the same multi-file fixture with one and twelve workers:

```rust
let mut serial = program_options(root);
serial.max_workers = Some(1);
let serial_result = build_local_graph(&serial)?;
let serial_graph = fs::read(serial_result.output_dir.join("graph.json"))?;
let serial_program = fs::read(serial_result.output_dir.join("program.json"))?;

fs::remove_dir_all(&serial_result.output_dir)?;
let mut parallel = program_options(root);
parallel.max_workers = Some(12);
let parallel_result = build_local_graph(&parallel)?;
assert_eq!(
    fs::read(parallel_result.output_dir.join("graph.json"))?,
    serial_graph
);
assert_eq!(
    fs::read(parallel_result.output_dir.join("program.json"))?,
    serial_program
);
```

Add a publication failure fixture that makes one destination unwritable and
asserts the incomplete marker remains and no new trusted state is accepted.

- [ ] **Step 2: Run tests before refactoring**

Run:

```bash
cargo test -p compass-core --test program_pipeline worker_count
cargo test -p compass-core --test pipeline_edge_coverage publication_failure
```

Expected: determinism may already pass; the failure-state assertion establishes
the refactor's safety gate.

- [ ] **Step 3: Extract graph assembly into a value-returning function**

Move resolve/build/cluster/analyze preparation into:

```rust
pub(crate) struct GraphAssembly {
    pub document: GraphDocument,
    pub communities: Communities,
    pub labels: BTreeMap<usize, String>,
    pub analysis: serde_json::Value,
    pub report: Option<String>,
    pub overview: Option<serde_json::Value>,
    pub stats: GraphStats,
}
```

`assemble_graph` performs no final output writes. Preserve semantic shrink
guards, previous community remapping, hub/cohesion/surprise/question logic, and
purpose-specific report values.

- [ ] **Step 4: Run Program assembly beside graph assembly**

Use a scoped thread:

```rust
let (graph, program) = std::thread::scope(|scope| {
    let program = options.program_analysis.then(|| {
        scope.spawn(|| build_program_from_evidence(root, &files, options, &cache))
    });
    let graph = assemble_graph(/* existing graph inputs */);
    let program = match program {
        Some(handle) => Some(
            handle
                .join()
                .map_err(|_| CoreError::WorkerPanic("program assembly".to_owned()))??,
        ),
        None => None,
    };
    Ok::<_, CoreError>((graph?, program))
})?;
```

Add the explicit `WorkerPanic` error variant. Avoid nested host-sized pools:
combined per-file extraction owns the Rayon pool; graph and Program assembly
use their existing internal parallel iterators without creating another full
pool.

Measure the two branch durations independently and store them in
`BuildTimings`.

- [ ] **Step 5: Publish independent artifacts concurrently**

In `publish.rs`, define:

```rust
pub(crate) struct PublishedArtifacts {
    pub graph: ArtifactSeal,
    pub program: Option<ArtifactSeal>,
    pub required: BTreeMap<String, ArtifactSeal>,
}
```

Use scoped threads to write graph, Program, every pending graph/Program cache
record, and small report artifacts. Join every writer, then save manifest and
`.compass_build_state.json`, then commit the guard.

If any writer fails, return the error without publishing state or committing
the guard.

- [ ] **Step 6: Preserve unchanged and no-cluster branches**

Route clustered, no-cluster, semantic, supplemental, and migration paths
through `publish_generation`. Unchanged verified state remains a no-write
return except for requested removal of optional visualization artifacts.

Do not make `--no-viz` a prerequisite for the performance gate; Django exceeds
the HTML node limit and must still complete with the default behavior.

- [ ] **Step 7: Run core and output verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-core
cargo test -p compass-output
cargo test -p compass-graph
cargo test -p compass-analysis
```

Expected: worker counts produce identical artifacts, all purpose branches pass,
and publication failures remain incomplete and untrusted.

- [ ] **Step 8: Run a release cold build and inspect CPU utilization**

Run:

```bash
cargo build --release --locked -p compass-cli --bin compass
benchmark_output=$(mktemp -d /tmp/compass-concurrent.XXXXXX)
/usr/bin/time -lp target/release/compass extract \
  /Users/haipingfu/Github/django \
  --code-only --force --timing --no-viz \
  --out "$benchmark_output"
```

Verify counts before comparing time. Record built-in stages, real/user/system
time, and peak RSS.

- [ ] **Step 9: Commit concurrent assembly and publication**

```bash
git add crates/compass-core/src/publish.rs \
  crates/compass-core/src/lib.rs \
  crates/compass-core/src/pipeline.rs \
  crates/compass-core/src/program.rs \
  crates/compass-core/src/build_state.rs \
  crates/compass-core/tests/program_pipeline.rs \
  crates/compass-core/tests/pipeline_edge_coverage.rs
git commit -m "perf(core): overlap index assembly and publication"
```

### Task 8: Add the Django acceptance gate and finish the performance loop

**Files:**

- Create: `scripts/qualify_django_performance.sh`
- Modify: `scripts/test_release_scripts.sh`
- Modify: `docs/implementation/extraction-pipeline.md`
- Modify: `docs/reference/commands.md`
- Modify: `docs/reference/outputs.md`
- Modify: `CHANGELOG.md`

**Interfaces:**

- Consumes: release `compass`, built-in elapsed lines, Django Git root
- Produces: `django-performance.tsv`
- Produces: summary with sample, median, and maximum for init, force, and update
- Enforces: every sample `< 5.0`

- [ ] **Step 1: Add shell contract tests**

Extend `scripts/test_release_scripts.sh` to assert the qualification script:

- rejects `/`, the home directory, a non-Git directory, and an empty
  `DJANGO_ROOT`;
- accepts `DJANGO_SAMPLES=1` in a tiny fixture repository;
- creates its worktree only below a `mktemp -d` directory;
- leaves the source repository status byte-for-byte unchanged; and
- exits nonzero when a stub Compass measurement is `5.000000000`.

Use a stub executable that writes valid Compass summary/timing lines and a tiny
valid graph/Program fixture so tests do not run Django.

- [ ] **Step 2: Run script tests and confirm failure**

Run:

```bash
bash scripts/test_release_scripts.sh
```

Expected: failure because `qualify_django_performance.sh` is absent.

- [ ] **Step 3: Implement guarded qualification setup**

Create a Bash script with:

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
results="$qualification_root/django-performance.tsv"
```

Resolve `django_root` with `git rev-parse --show-toplevel`. Reject an empty
value, `/`, the user's home, and a non-Git path. Capture source status before
work. Add a detached worktree at the exact source commit and remove it in a
trap before deleting the validated temporary root.

- [ ] **Step 4: Measure the three workflows**

Use `scripts/measure_process.py` around:

```bash
COMPASS_OUT=compass-perf-out "$compass_bin" init . --yes --force --timing
COMPASS_OUT=compass-perf-out "$compass_bin" update . --force --timing
COMPASS_OUT=compass-perf-out "$compass_bin" update . --timing
```

For every cold init sample, remove only:

```text
$django_worktree/compass-perf-out
$django_worktree/.compass/config.toml
```

after verifying both have `$django_worktree` as their canonical parent/root.
Do not remove or modify any path in the user's source checkout.

Write TSV columns:

```text
operation sample seconds peak_kib files nodes edges communities modules summaries conflicts graph_sha256 program_sha256
```

Parse counts from Compass stdout and digests with `shasum -a 256`.

- [ ] **Step 5: Enforce latency and output equivalence**

Use an embedded Python summary that:

```python
for operation, rows in groups.items():
    seconds = [float(row["seconds"]) for row in rows]
    if any(value >= limit for value in seconds):
        raise SystemExit(
            f"{operation} exceeded {limit:.2f}s: "
            + ", ".join(f"{value:.3f}" for value in seconds)
        )
    print(
        operation,
        f"samples={len(seconds)}",
        f"median={statistics.median(seconds):.3f}s",
        f"max={max(seconds):.3f}s",
    )
```

Fail when file/node/edge/community/module/summary/conflict counts differ across
full builds. Compare canonical public artifact digests for repeated identical
operations; if operation-specific metadata differs, normalize only the
documented metadata field before hashing and explain it in output.

- [ ] **Step 6: Run the shell tests**

Run:

```bash
bash scripts/test_release_scripts.sh
```

Expected: safety, threshold, and source-status cases pass.

- [ ] **Step 7: Run full Django qualification**

Run:

```bash
cargo build --release --locked -p compass-cli --bin compass
DJANGO_ROOT=/Users/haipingfu/Github/django \
  COMPASS_BIN=target/release/compass \
  DJANGO_SAMPLES=3 \
  bash scripts/qualify_django_performance.sh
```

If a cold stage exceeds its budget, use the built-in timing evidence to optimize
that exact stage without changing output. Repeat all three workflows after each
optimization. Completion requires every sample, not only the median, below
5.0 seconds.

- [ ] **Step 8: Document the timing and qualification contracts**

In command reference docs, show:

```text
Compass update completed in 0.91s.
```

Document `--timing` on init/extract/update and state that overlapping stages do
not sum to total. In output docs, document `.compass_build_state.json` as an
internal integrity seal, MessagePack caches as disposable internal data, and
public JSON as unchanged.

In extraction implementation docs, add the exact qualification command and
the one-read/one-parse pipeline diagram.

- [ ] **Step 9: Run repository-wide verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
bash scripts/test_release_scripts.sh
graphify update .
git diff --check
git status --short
```

Expected: all checks pass and only intentional tracked changes plus the required
untracked `graphify-out/` refresh are present.

- [ ] **Step 10: Commit qualification and documentation**

```bash
git add scripts/qualify_django_performance.sh \
  scripts/test_release_scripts.sh \
  docs/implementation/extraction-pipeline.md \
  docs/reference/commands.md \
  docs/reference/outputs.md \
  CHANGELOG.md
git commit -m "test(perf): gate Django indexing below five seconds"
```

- [ ] **Step 11: Push and update the pull request**

Run:

```bash
git push origin codex/fix-python-symbol-collisions
gh pr edit 46 \
  --title "Fix repeated Python symbols and accelerate Django indexing"
gh pr comment 46 --body-file "$qualification_summary"
```

Add a PR comment containing:

- the three samples for each operation;
- medians and maxima;
- exact Django commit and machine CPU count;
- output counts and canonical digests;
- focused and workspace test commands; and
- the built-in timing output.

Do not mark the PR ready until every acceptance criterion is evidenced.

---

## Plan self-review

- Spec coverage: timing, one read, one parse, cache format, state seal,
  corruption recovery, concurrent assembly/publication, output equivalence,
  qualification, documentation, graph refresh, commit, push, and PR evidence
  each map to a task.
- Placeholder scan: no deferred implementation markers remain.
- Type consistency: `ArtifactSeal`, `BuildState`, `SourceSnapshot`,
  `DeterministicFileResult`, `ProgramBuild::canonical_bytes`,
  `BuildTimings`, and `PublishedArtifacts` have one definition and consistent
  consumers.
