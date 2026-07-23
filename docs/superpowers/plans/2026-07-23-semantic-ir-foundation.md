# Program IR Evidence Foundation Implementation Plan

> Historical note: this plan describes the original `compass.program/1`
> foundation. Current output uses `compass.program/2` with complete, partial,
> indeterminate, and failed coverage states while retaining schema-1 read
> compatibility. Current artifact status also reports decoded artifacts and
> per-document analysis/reuse separately.

> **For implementers:** execute this plan task by task with
> `superpowers:executing-plans`. Use red-green-refactor for every task and run
> the stated verification before each commit.

**Goal:** Make native Compass builds emit, cache, validate, summarize, and
historically preserve a deterministic `program.json` assembled from
complementary program-evidence providers.

**Architecture:** `compass-ir` owns the provider-neutral schema.
`compass-program` owns provider contracts, official SCIP decoding, and
deterministic evidence reconciliation. `compass-languages` provides the initial
Tree-sitter syntax providers for Rust and TypeScript/JavaScript.
`compass-analysis` derives behavior summaries from the merged Program IR.
`compass-core` orchestrates the provider scopes, caches evidence independently,
and atomically installs the final artifact. Tree-sitter is the zero-configuration
syntax baseline, not the universal semantic engine.

**Tech stack:** Rust 1.97.1, Rust 2024 edition, Serde/JSON, SHA-256,
Tree-sitter 0.26, official `scip = "=0.9.0"` protobuf bindings, bounded
protobuf decoding, Rayon, `prolly-map = "=0.5.0"`, and
`prolly-store-sqlite = "=0.3.0"`.

## Decisions fixed by this plan

- The only native artifact name is `compass-out/program.json`. Never create,
  read, migrate, reserve, or document `.compass_program.json`.
- Program IR is a source-oriented evidence model inspired by LLVM's stable
  intermediate layer; it is not LLVM IR and does not reduce all languages to
  one instruction set.
- The first provider stack is:
  1. Tree-sitter syntax evidence for Rust, TypeScript, TSX, and JavaScript;
  2. optional official SCIP protobuf evidence already present on disk;
  3. a project-analyzer contract with no production compiler integration yet.
- Compass never invokes an indexer, compiler, language server, network service,
  or model in this foundation.
- Native `compass update`, `extract`, and `watch` enable program analysis.
  Graphify compatibility mode preserves its current file set and output.
- A repository-root `index.scip` is discovered automatically. The repeatable
  native CLI option `--program-artifact <PATH>` adds explicit artifacts.
  Explicit missing paths are errors; an absent conventional artifact is not.
- A raw SCIP index has no trustworthy source-content digest. Compass uses its
  facts but marks affected semantic capabilities partial with
  `artifact_revision_unverified`.
- An optional companion file `<artifact>.compass-manifest.json` can prove
  freshness. It contains the index SHA-256 and normalized path-to-source-digest
  entries. A manifest/index digest mismatch is fatal. A source digest mismatch
  excludes only that document and records `stale_artifact_document`.
- Syntax evidence owns source structure, operation order, and exact spans.
  SCIP may enrich symbol identity, definitions, references, roles,
  implementations, types, and call targets. Authority is per capability.
- Evidence conflicts are retained and lower only the affected capability's
  coverage. Input order must never decide a conflict.
- Unsupported source files produce no program module. SCIP for an unsupported
  language continues through the existing structural graph ingestion path but
  does not fabricate a Program IR body.
- Absolute checkout paths, SCIP `project_root`, timestamps, protobuf field
  order, provider input order, and cache location never enter canonical output.
- Incremental and clean builds of the same logical inputs must emit identical
  `program.json` bytes.
- Program facts and summaries are authoritative historical state in their own
  Prolly roots.
- After code changes, run `compass update .` in this repository and then
  `graphify update .` in `/Users/haipingfu/graphify`.

## Scope boundary

This plan delivers the evidence ingestion and normalization foundation. It does
not implement branch-complete CFGs, interprocedural fixed-point data flow,
native compiler integrations, runtime overlays, semantic impact, repository
federation, or agent APIs.

The first artifact schema is `compass.program/1`. It contains:

- the exact provider manifest used for the build;
- evidence provenance and capability-specific coverage;
- normalized modules, functions, source anchors, operations, and resolutions;
- deterministic function summaries and reverse-call dependencies.

The foundation deliberately reports syntax-only limitations. A Tree-sitter
provider may report complete syntax extraction and partial or unavailable type,
call-resolution, CFG, data-flow, effect, and contract coverage at the same time.

## Dependency direction and file ownership

```text
compass-ir
    ^
    |
compass-program <----- compass-languages
    ^                         ^
    |                         |
compass-analysis         compass-core
    ^                         |
    +-------------------------+
                              |
                       compass-history
```

`compass-program` must not depend on `compass-languages` or `compass-core`.
`compass-languages` implements provider traits defined by `compass-program`.

### New crates

- `crates/compass-ir/`: pure schema, canonicalization, validation, and digests.
- `crates/compass-program/`: provider contracts, evidence model, SCIP decoder,
  merge rules, and conflict diagnostics.
- `crates/compass-analysis/`: immutable behavior summaries and invalidation.

### Existing crates

- `crates/compass-languages/src/program/`: Tree-sitter syntax providers.
- `crates/compass-files/src/cache.rs`: isolated syntax and artifact caches.
- `crates/compass-core/src/program.rs`: discovery, orchestration, merge,
  summarization, and atomic output.
- `crates/compass-history/`: Program IR and summary Prolly roots.
- `crates/compass-cli/`: native artifact options and stable status reporting.
- `crates/compass-output/src/backup.rs`: protected-output backup registration.

The existing `compass-languages/src/scip.rs` simplified-JSON structural graph
ingestor remains unchanged. Official protobuf Program IR ingestion is a
separate implementation in `compass-program`.

## Public interfaces fixed by this plan

The names may be reorganized internally, but these semantic contracts must not
change during implementation.

```rust
// compass-ir
pub const PROGRAM_SCHEMA: &str = "compass.program/1";
pub const PROGRAM_SCHEMA_VERSION: u32 = 1;

pub type EvidenceId = String;
pub type SymbolId = String;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Syntax,
    Artifact,
    Project,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Syntax,
    SymbolIdentity,
    Definitions,
    References,
    Types,
    CallResolution,
    ControlFlow,
    DataFlow,
    Effects,
    Contracts,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CoverageState {
    Complete,
    Partial { reasons: Vec<String> },
    Unavailable { reasons: Vec<String> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub kind: ProviderKind,
    pub version: String,
    pub scope: String,
    pub input_digest: String,
    pub configuration_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: EvidenceId,
    pub provider_id: String,
    pub source_file: Option<String>,
    pub capability: Capability,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramBundle {
    pub schema: String,
    pub providers: Vec<ProviderDescriptor>,
    pub evidence: Vec<EvidenceRecord>,
    pub modules: Vec<ModuleIr>,
}
```

Every `ModuleIr`, `FunctionIr`, `Operation`, type resolution, and call
resolution has a sorted, deduplicated `evidence: Vec<EvidenceId>`. Modules and
functions have `coverage: BTreeMap<Capability, CoverageState>`. A
`SourceAnchor` contains only a normalized repository-relative path and a
half-open UTF-8 byte span. Syntax calls store the callee identifier span
separately from the full expression span so semantic occurrences can be joined
without heuristics.

```rust
// compass-program
pub struct FileInput<'a> {
    pub source_file: &'a str,
    pub language: &'a str,
    pub source: &'a [u8],
}

pub struct ArtifactInput<'a> {
    pub logical_name: &'a str,
    pub input_digest: &'a str,
    pub byte_len: u64,
    pub manifest: Option<&'a ArtifactManifest>,
    pub source_digests: &'a BTreeMap<String, String>,
    pub limits: ArtifactLimits,
}

pub struct ProjectInput<'a> {
    pub repository_digest: &'a str,
    pub build_context_digest: &'a str,
    pub files: &'a [ProjectFile<'a>],
}

pub trait SyntaxProvider {
    fn descriptor(&self, input: &FileInput<'_>) -> ProviderDescriptor;
    fn analyze_file(
        &mut self,
        input: FileInput<'_>,
    ) -> Result<Option<EvidenceBatch>, ProviderError>;
}

pub trait ArtifactProvider {
    fn descriptor(&self, input: &ArtifactInput<'_>) -> ProviderDescriptor;
    fn analyze_artifact(
        &self,
        input: ArtifactInput<'_>,
        reader: &mut dyn ArtifactReader,
    ) -> Result<EvidenceBatch, ProviderError>;
}

pub trait ProjectAnalyzer {
    fn descriptor(
        &self,
        repository_digest: &str,
        build_context_digest: &str,
    ) -> ProviderDescriptor;
    fn analyze_project(
        &self,
        input: ProjectInput<'_>,
    ) -> Result<EvidenceBatch, ProviderError>;
}

pub fn merge_evidence(
    batches: Vec<EvidenceBatch>,
) -> Result<ProgramBundle, MergeError>;

pub trait ArtifactReader: std::io::Read + std::io::Seek {}
impl<T: std::io::Read + std::io::Seek> ArtifactReader for T {}
```

`EvidenceBatch` contains one descriptor, scoped facts, and declared coverage.
It is validated before merging. `merge_evidence` sorts batches by the canonical
descriptor tuple and produces the same bytes for every permutation. Descriptor
IDs identify one provider invocation, not merely an implementation: syntax IDs
include logical file scope and input digest, artifact IDs include artifact
digest, and project IDs include repository and build-context digests.
`ArtifactInput.logical_name` is diagnostic-only and is never copied into a
descriptor or canonical evidence.

```rust
// compass-analysis
pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;
pub const ANALYZER_VERSION: u32 = 1;

pub struct AnalysisBundle {
    pub program: ProgramBundle,
    pub summaries: Vec<FunctionSummary>,
    pub reverse_calls: BTreeMap<SymbolId, Vec<SymbolId>>,
}

pub fn analyze(program: ProgramBundle) -> Result<AnalysisBundle, AnalysisError>;
```

## Task 1: Add the provenance-aware Program IR schema

**Files:**

- Modify: `Cargo.toml`
- Create: `crates/compass-ir/Cargo.toml`
- Create: `crates/compass-ir/src/lib.rs`
- Create: `crates/compass-ir/src/model.rs`
- Create: `crates/compass-ir/src/canonical.rs`
- Create: `crates/compass-ir/src/validation.rs`
- Create: `crates/compass-ir/tests/schema.rs`

### Step 1: Write failing schema tests

Add tests that construct two logically identical bundles with reversed
providers, evidence, modules, functions, blocks, operations, coverage reasons,
and evidence IDs. Assert:

- `canonical_bytes()` are identical;
- `digest()` is identical;
- canonical output contains `compass.program/1`;
- absolute paths and unknown evidence IDs fail validation;
- duplicate provider IDs and duplicate evidence IDs fail validation;
- an operation may be syntax-complete while call resolution is partial;
- every declared `EvidenceId` resolves to a record from a registered provider.

Run:

```bash
cargo test -p compass-ir --test schema
```

Expected: Cargo reports that `compass-ir` does not exist.

### Step 2: Create the crate and schema

Add the crate to the workspace. Use only workspace `serde`, `serde_json`,
`sha2`, and `thiserror`. Implement the public types above plus:

- `ModuleIr`, `FunctionIr`, `BasicBlock`, `Operation`, and `Terminator`;
- `SourceAnchor { source_file, start_byte, end_byte }`;
- `OperationKind::{Call, Read, Write, Await, Throw}`;
- optional semantic identity and resolution fields without a provider-specific
  payload;
- `ProgramBundle::validate`, `canonicalized`, `canonical_bytes`, and `digest`.

Canonicalization sorts and deduplicates every set-like field. It preserves
source order only through explicit operation ordinals. Validation rejects:

- noncanonical, absolute, empty, `.` or `..` source paths;
- invalid or overlapping half-open byte spans where nesting is not allowed;
- missing evidence and provider references;
- duplicate IDs;
- block edges to missing block IDs;
- resolved call targets without call-resolution evidence;
- `Complete` coverage with a nonempty reason list.

### Step 3: Prove checkout-root and provider-order independence

Add fixtures whose logical paths and content are identical but whose temporary
checkout roots differ. Add all `3!` permutations of three providers and assert
one digest. Ensure provider `input_digest` changes the result while the
artifact's absolute path does not.

Run:

```bash
cargo test -p compass-ir
cargo clippy -p compass-ir --all-targets -- -D warnings
```

Expected: all tests pass.

### Step 4: Commit

```bash
git add Cargo.toml Cargo.lock crates/compass-ir
git commit -m "feat(ir): add provenance-aware Program IR schema"
```

## Task 2: Add provider contracts and deterministic evidence merge

**Files:**

- Modify: `Cargo.toml`
- Create: `crates/compass-program/Cargo.toml`
- Create: `crates/compass-program/src/lib.rs`
- Create: `crates/compass-program/src/provider.rs`
- Create: `crates/compass-program/src/evidence.rs`
- Create: `crates/compass-program/src/merge.rs`
- Create: `crates/compass-program/src/path.rs`
- Create: `crates/compass-program/tests/merge.rs`

### Step 1: Write merge-contract tests

Create fixture syntax, artifact, and project batches for one call. Assert:

1. syntax supplies the module, function, operation order, expression span, and
   callee span;
2. artifact evidence upgrades the callee to a semantic `SymbolId`;
3. project evidence may add a type without replacing either source span;
4. all six batch permutations produce byte-identical Program IR;
5. two incompatible resolved targets are both retained and mark only
   `CallResolution` partial with `provider_conflict`;
6. an artifact occurrence without an unambiguous source anchor remains
   unattached evidence and marks `unmatched_semantic_occurrence`;
7. a fake `ProjectAnalyzer` can emit a batch without filesystem or process
   access.

Run:

```bash
cargo test -p compass-program --test merge
```

Expected: the package does not exist.

### Step 2: Implement provider inputs and path safety

Add `compass-ir`, `serde`, `serde_json`, `sha2`, and `thiserror`.
`FileInput` and evidence batches accept logical paths only. Implement one path
normalizer shared by the merger and later SCIP decoder:

- convert separators to `/`;
- reject absolute paths, drive prefixes, NUL, empty components, `.`, and `..`;
- reject paths that normalize to the output directory or program cache;
- preserve case and Unicode bytes;
- return a typed `ProviderError::UnsafePath`.

`ProjectInput` is an in-memory contract only. It contains normalized files and
digests, not a checkout root or command runner.

Generate every `EvidenceId` as lowercase SHA-256 over canonical bytes for
`(provider_id, capability, normalized_path, anchor, fact_kind, fact_payload)`.
The configuration digest includes only meaning-affecting normalized settings.
For SCIP it distinguishes raw from companion-verified input by including the
companion manifest digest; it never includes resource limits or filesystem
paths. Provider algorithm and grammar changes increment the provider version.

### Step 3: Implement capability-specific merge rules

Implement these rules in `merge.rs`:

- create source structure only from syntax or future project evidence that
  explicitly declares source structure;
- join semantic facts by exact source file and callee/identifier span first;
- use a smallest-enclosing occurrence only when exactly one candidate exists;
- prefer no result over a guessed match;
- union identical facts and their evidence IDs;
- retain incompatible facts and add deterministic conflict evidence;
- compute module/function coverage per capability from attached facts and
  provider declarations;
- canonicalize after reconciliation, never before conflict detection;
- reject a provider that claims facts outside its declared scope.

Do not implement one global provider precedence number.

### Step 4: Add property tests for order and idempotence

Without adding a random dependency, generate deterministic permutations and
duplicate-batch cases. Assert:

```text
merge(permutation(inputs)) == merge(inputs)
merge(inputs + duplicate_identical_batch) == merge(inputs)
merge(canonicalized_inputs) == merge(inputs)
```

Run:

```bash
cargo test -p compass-program
cargo clippy -p compass-program --all-targets -- -D warnings
```

Expected: all tests pass.

### Step 5: Commit

```bash
git add Cargo.toml Cargo.lock crates/compass-program
git commit -m "feat(program): add evidence providers and deterministic merge"
```

## Task 3: Add deterministic summaries and invalidation

**Files:**

- Modify: `Cargo.toml`
- Create: `crates/compass-analysis/Cargo.toml`
- Create: `crates/compass-analysis/src/lib.rs`
- Create: `crates/compass-analysis/src/summary.rs`
- Create: `crates/compass-analysis/src/invalidation.rs`
- Create: `crates/compass-analysis/tests/summary.rs`

### Step 1: Write failing summary tests

Construct merged Program IR rather than provider-specific modules. Test:

- calls, reads, writes, awaits, throws, and unresolved calls are summarized;
- summary evidence is the sorted union of supporting operation evidence;
- capability coverage propagates without collapsing into one global state;
- reverse calls use only unambiguous resolved targets;
- provider-conflicted calls do not enter the definitive reverse-call index;
- changing one body digest invalidates its summary and the reverse resolved
  callers, while changing unattached evidence invalidates nothing;
- clean recomputation equals incremental recomputation byte for byte.

Run:

```bash
cargo test -p compass-analysis --test summary
```

Expected: the package does not exist.

### Step 2: Implement summaries

Add `compass-ir` and no dependency on provider implementations. Define
`FunctionSummary` with symbol, body digest, calls, reads, writes, effects,
errors, evidence, coverage, and summary digest. Keep unresolved call text
separate from resolved `SymbolId` values.

`AnalysisBundle::canonical_bytes()` validates the embedded Program IR, sorts
summaries and reverse dependencies, and uses the same canonical JSON rules as
`compass-ir`.

### Step 3: Implement invalidation

Expose:

```rust
pub fn affected_summaries(
    previous: &AnalysisBundle,
    current: &ProgramBundle,
) -> Result<BTreeSet<SymbolId>, AnalysisError>;
```

Compare semantic function digests, then walk the previous and current reverse
resolved-call indexes to a fixed point. Provider manifest changes alone do not
invalidate every summary; only changed merged function facts do.

Run:

```bash
cargo test -p compass-analysis
cargo clippy -p compass-analysis --all-targets -- -D warnings
```

Expected: all tests pass.

### Step 4: Commit

```bash
git add Cargo.toml Cargo.lock crates/compass-analysis
git commit -m "feat(analysis): derive evidence-backed function summaries"
```

## Task 4: Add scope-correct program caches

**Files:**

- Modify: `crates/compass-files/src/cache.rs`
- Modify: `crates/compass-files/src/lib.rs`
- Modify: `crates/compass-files/tests/contracts.rs`

### Step 1: Write failing cache isolation tests

Test three independent namespaces:

```rust
CacheKind::ProgramSyntax {
    ir_schema: 1,
    provider_version: "tree-sitter-rust/1".to_owned(),
}
CacheKind::ProgramArtifact {
    ir_schema: 1,
    decoder_version: "scip/1".to_owned(),
}
CacheKind::ProgramMerge {
    ir_schema: 1,
    merger_version: 1,
    analyzer_version: 1,
}
```

Assert:

- syntax keys include normalized relative path plus source digest;
- same bytes at different logical paths do not alias;
- artifact keys include artifact digest and decoder version;
- changing `index.scip` does not evict syntax entries;
- merge keys include the canonical provider-manifest digest;
- corrupt JSON is ignored and recomputed;
- `clear_all` and the existing cache cleanup include all three namespaces.

Run:

```bash
cargo test -p compass-files --test contracts program_cache
```

Expected: compilation fails because the variants do not exist.

### Step 2: Add logical-input cache APIs

The current cache API hashes filesystem paths and absolutizes graph source
fields. Do not reuse that behavior for Program IR. Add:

```rust
pub fn load_program<T: DeserializeOwned>(
    &self,
    kind: &CacheKind,
    logical_key: &str,
) -> Result<Option<T>, FileError>;

pub fn save_program<T: Serialize>(
    &self,
    kind: &CacheKind,
    logical_key: &str,
    value: &T,
) -> Result<(), FileError>;
```

Hash the UTF-8 logical key with SHA-256. Program cache values must never pass
through `absolutize_source_files`. Save atomically. Validate loaded values in
their owning crate; a cache hit is not trusted merely because JSON parsed.

### Step 3: Add pruning by live logical key

Add namespace-specific pruning after a successful provider phase. Never prune
syntax entries after artifact failure, and never prune artifact shards after
syntax failure. Add interrupted-write and concurrent-reader coverage.

Run:

```bash
cargo test -p compass-files
cargo clippy -p compass-files --all-targets -- -D warnings
```

Expected: all tests pass.

### Step 4: Commit

```bash
git add crates/compass-files
git commit -m "feat(files): add isolated program evidence caches"
```

## Task 5: Implement the Tree-sitter syntax baseline

**Files:**

- Modify: `crates/compass-languages/Cargo.toml`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/engine.rs`
- Create: `crates/compass-languages/src/program/mod.rs`
- Create: `crates/compass-languages/src/program/rust.rs`
- Create: `crates/compass-languages/src/program/typescript.rs`
- Create: `crates/compass-languages/tests/program_evidence.rs`

### Step 1: Write failing provider tests

Use `TreeSitterSyntaxProvider::default()` through the `SyntaxProvider` trait.
Add Rust and TypeScript fixtures containing functions, methods, calls, reads,
writes, `await`, `throw`/panic, branches, and ambiguous dispatch.

Assert:

- supported files return one valid `EvidenceBatch`;
- unsupported Go returns `None`;
- exact operation and callee spans slice the expected source bytes;
- syntax, definitions, and lexical effects have syntax evidence IDs;
- types and call resolution are partial or unavailable rather than invented;
- a uniquely resolvable same-module call may carry a conservative local target;
- traits, imports, virtual calls, dynamic access, macros, decorators, JSX, and
  branch-sensitive CFGs add exact capability reasons;
- two identical files at different logical paths have distinct symbol IDs;
- all TypeScript-family registry extensions dispatch correctly.

Run:

```bash
cargo test -p compass-languages --test program_evidence
```

Expected: `TreeSitterSyntaxProvider` is undefined.

### Step 2: Add the provider dispatch

Add dependencies on `compass-ir` and `compass-program`. Export:

```rust
pub const TREE_SITTER_PROGRAM_PROVIDER_VERSION: u32 = 1;
pub struct TreeSitterSyntaxProvider {
    engine: Engine,
}
```

Implement `SyntaxProvider` by resolving the existing `LanguageSpec`, parsing
with the existing statically linked grammar, and dispatching to the Rust or
TypeScript-family module. Do not expose the old proposed
`Engine::program_ir_source` API; providers emit `EvidenceBatch`, not final IR.

Map invalid output to a typed `ExtractError::InvalidProgramEvidence` containing
the logical path and validation detail.

### Step 3: Implement Rust syntax evidence

Reuse existing Tree-sitter nodes and structural graph identity helpers. Emit:

- functions and `impl` methods;
- stable symbols based on logical path, qualified owner, name, and signature
  digest;
- signature and body digests;
- a conservative entry block and source-ordered operations;
- exact identifier spans for calls and exact expression spans for operations;
- reads, writes, awaits, explicit returns, panic/error macro calls;
- unique same-module call resolution only when provable from syntax.

Mark capability-specific reasons including:

```text
branch_sensitive_cfg
question_mark_control_flow
macro_expansion_unavailable
trait_dispatch_unresolved
reflection_unresolved
graph_identity_collision
```

Do not mark type or data-flow coverage complete.

### Step 4: Implement TypeScript-family syntax evidence

Cover functions, methods, variable-bound arrow functions, calls, property
reads, writes, `await`, `throw`, and explicit returns for `.ts`, `.mts`, `.cts`,
`.tsx`, `.js`, `.jsx`, `.mjs`, and `.cjs`.

Use exact reasons:

```text
compiler_types_unavailable
import_resolution_unavailable
dynamic_property_access
prototype_mutation
decorator_semantics
eval_or_function_constructor
branch_sensitive_cfg
exception_flow
jsx_framework_dispatch
```

### Step 5: Verify structural extraction did not change

Run:

```bash
cargo test -p compass-languages --test program_evidence
cargo test -p compass-languages
cargo clippy -p compass-languages --all-targets -- -D warnings
```

Expected: all new tests pass and existing graph fixtures remain byte-equivalent.

### Step 6: Commit

```bash
git add crates/compass-languages
git commit -m "feat(languages): add Tree-sitter program evidence providers"
```

## Task 6: Ingest official SCIP as an offline artifact provider

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/compass-program/Cargo.toml`
- Modify: `crates/compass-program/src/lib.rs`
- Create: `crates/compass-program/src/scip.rs`
- Create: `crates/compass-program/src/scip_stream.rs`
- Create: `crates/compass-program/src/manifest.rs`
- Create: `crates/compass-program/tests/scip.rs`
- Create: `crates/compass-program/tests/support/scip_fixture.rs`

### Step 1: Add official SCIP fixtures and failing tests

Build fixtures in memory from the official generated message types and serialize
them with `Message::write_to_bytes`; pass the bytes through `Cursor<Vec<u8>>`.
The support module also writes top-level fields in alternate valid wire orders
for the order-independence case. Tests must cover:

- metadata and document path normalization;
- UTF-8 and UTF-16 occurrence-range conversion to UTF-8 byte spans;
- definitions, references, imports, reads, writes, implementations, type
  definitions, symbol kind, and enclosing range;
- symbol and relationship identity without copying `project_root`;
- unknown protobuf fields;
- raw index coverage reason `artifact_revision_unverified`;
- valid companion manifest removes that reason;
- stale source digest skips only the stale document;
- companion index-digest mismatch fails the artifact;
- absolute, parent-traversing, duplicate-normalized, and output-directory
  document paths fail;
- malformed, truncated, oversized-document, excessive-record, and unsupported
  text-encoding cases return typed errors;
- protobuf field order and input document order do not affect normalized batch
  bytes.

Run:

```bash
cargo test -p compass-program --test scip
```

Expected: `OfficialScipProvider` is undefined.

### Step 2: Pin official bindings and implement bounded streaming

Add workspace `scip = "=0.9.0"` and the exact compatible `protobuf` version
selected into `Cargo.lock`. Do not shell out to `scip`, an indexer, or `protoc`.

Define:

```rust
pub const SCIP_PROVIDER_VERSION: u32 = 1;

pub struct ArtifactLimits {
    pub max_artifact_bytes: u64,   // default 2 GiB
    pub max_document_bytes: u64,   // default 64 MiB
    pub max_metadata_bytes: u64,   // default 8 MiB
    pub max_records: u64,          // default 50,000,000
}
```

Read the top-level protobuf wire format in safe Rust and decode one length-
delimited `Document` at a time with official message types. Use a seekable
two-pass reader so metadata and document encoding are known even when protobuf
fields arrive in a different order: the first pass validates metadata and
external symbols; the second normalizes documents. Retain external symbols only
as bounded normalized maps. Do not deserialize the whole index into one object
or read it into one `Vec<u8>`. Check the file length before reading and
checked-add all record counters.

### Step 3: Implement the companion manifest

Schema:

```json
{
  "schema": "compass.scip-manifest/1",
  "index_sha256": "<lowercase hex>",
  "documents": {
    "src/lib.rs": "<source sha256>"
  }
}
```

Canonicalize document paths through the shared path validator. Reject duplicate
normalized paths, malformed digests, unknown schema versions, and an index
digest mismatch. A missing document digest means freshness is unverified for
that document; a mismatched digest means skip its semantic facts and emit
`stale_artifact_document`.

### Step 4: Normalize SCIP facts

Map official fields into provider-neutral evidence:

- `Document.relative_path` to normalized source file;
- occurrence ranges to `SourceAnchor`;
- occurrence symbol and roles to definitions/references/import/read/write;
- `SymbolInformation.relationships` to implementation, type-definition,
  definition, and reference facts;
- symbol documentation/signature only as bounded evidence detail, not as
  Program IR source structure;
- metadata tool name/version into evidence detail while descriptor version
  remains Compass's decoder version.

Ignore SCIP `project_root` after validating metadata shape. Never hash it into
the descriptor. Descriptor `input_digest` is the exact artifact SHA-256.

### Step 5: Test merge enrichment

Combine the Rust/TypeScript syntax fixtures with SCIP evidence and assert:

- the module and operation sequence are unchanged;
- definitions, references, symbol identity, and call resolution improve;
- explicit SCIP type-definition relationships add type-definition evidence,
  while general expression-type coverage remains partial;
- every improved fact cites both its syntax anchor and SCIP evidence when both
  support it;
- conflicting targets stay visible and lower call-resolution coverage;
- SCIP for a Go-only fixture creates no Program IR module.

Run:

```bash
cargo test -p compass-program
cargo test -p compass-languages --test program_evidence
cargo clippy -p compass-program --all-targets -- -D warnings
```

Expected: all tests pass.

### Step 6: Commit

```bash
git add Cargo.toml Cargo.lock crates/compass-program
git commit -m "feat(program): ingest official SCIP evidence"
```

## Task 7: Orchestrate providers and atomically emit `program.json`

**Files:**

- Modify: `crates/compass-core/Cargo.toml`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Create: `crates/compass-core/src/program.rs`
- Create: `crates/compass-core/tests/program_pipeline.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Create: `crates/compass-cli/tests/program_cli.rs`
- Modify: `crates/compass-output/src/backup.rs`

### Step 1: Write end-to-end failing tests

Create a repository with Rust and TypeScript source plus `index.scip` and a
freshness manifest. Test:

1. cold native build emits canonical `compass-out/program.json`;
2. warm build reuses syntax and artifact caches;
3. changing only SCIP reuses syntax but decodes/merges artifact evidence;
4. changing one TypeScript file reanalyzes one syntax entry and excludes a
   stale SCIP document;
5. deleting a source removes its module;
6. two checkout roots produce byte-identical artifacts;
7. reversing explicit artifact arguments does not change output;
8. malformed explicit or discovered SCIP fails before replacing the previous
   valid artifact and leaves the build-incomplete marker;
9. a directory obstructing `program.json` fails atomically;
10. Graphify compatibility emits no `program.json` and no new status text.

Run:

```bash
cargo test -p compass-core --test program_pipeline
cargo test -p compass-cli --test program_cli
```

Expected: `BuildOptions` lacks program fields.

### Step 2: Add build inputs and result metrics

Extend `BuildOptions`:

```rust
pub program_analysis: bool,
pub program_artifacts: Vec<PathBuf>,
pub program_artifact_limits: compass_program::ArtifactLimits,
```

Default `program_analysis` to `false` for embedders. Extend `BuildResult`:

```rust
pub program_modules: usize,
pub program_summaries: usize,
pub program_syntax_analyzed: usize,
pub program_syntax_reused: usize,
pub program_artifacts_loaded: usize,
pub program_artifacts_reused: usize,
pub program_conflicts: usize,
```

Set `program_analysis = true` only at native Compass update/extract/watch
frontend boundaries. Compatibility frontends keep it false.

### Step 3: Implement artifact discovery

In `compass-core/src/program.rs`:

- discover `root/index.scip` when it is a regular file;
- append explicit paths, resolve them once, and require regular files;
- reject output/cache paths and paths outside the repository unless explicitly
  supplied;
- use the explicit artifact's filename only as a logical name;
- sort by content digest and deduplicate byte-identical artifacts;
- use provider IDs derived from format plus content digest, never argument
  position or filename;
- discover the exact companion filename
  `<artifact>.compass-manifest.json`;
- open a seekable bounded reader, validate file length, and compute the digest
  without retaining the entire artifact in memory.

An explicit outside-repository artifact is allowed, but its absolute path never
enters evidence, cache keys, errors serialized into `program.json`, or history.

### Step 4: Implement scoped orchestration

Expose internally:

```rust
pub(crate) const PROGRAM_ARTIFACT: &str = "program.json";

pub(crate) fn build_program(
    root: &Path,
    sources: &[PathBuf],
    options: &BuildOptions,
    cache: &mut compass_files::Cache,
) -> Result<ProgramBuild, CoreError>;

pub(crate) fn write_program(
    output_dir: &Path,
    analysis: &compass_analysis::AnalysisBundle,
) -> Result<(), CoreError>;
```

`build_program` performs:

1. normalize source paths and compute source digests;
2. load or produce per-file syntax evidence;
3. load or decode each artifact into document-sharded evidence;
4. validate every cached batch before use;
5. merge all live batches with `merge_evidence`;
6. analyze the merged Program IR;
7. cache the merge by canonical provider-manifest digest;
8. prune only namespaces whose provider phase completed.

Use the existing sequential-under-256 and build-local bounded Rayon policy for
syntax files. Decode artifacts sequentially in canonical order in this
foundation; document streaming bounds memory.

### Step 5: Preserve unchanged and atomic build paths

Before the earliest manifest-unchanged return, validate existing
`program.json`:

- canonical bytes exactly match the parsed value;
- all schema/analyzer versions are current;
- provider manifest matches current source/artifact discovery and digests;
- embedded evidence and IR validate.

If valid, return counts without opening program caches. If invalid or absent,
rebuild only the program phase; do not force AST extraction.

On every successful clustered, raw, topology-unchanged, and incremental branch,
write canonical bytes with the existing atomic byte writer immediately before
the build guard commits. Never independently serialize the value a second way.

Malformed provider input, fresh provider failure, invalid merged IR, or output
failure must leave the previous valid artifact untouched.

### Step 6: Add native CLI options and stable output

Add the repeatable native-only option:

```text
--program-artifact <PATH>
```

Support update, extract, and watch. Reject it in Graphify compatibility mode
with a clear unsupported-option error rather than silently ignoring it.

Print one native line:

```text
Program analysis: <syntax_analyzed> syntax analyzed, <syntax_reused> syntax reused, <artifacts_loaded> artifacts loaded, <artifacts_reused> artifacts reused, <artifact_documents_analyzed> artifact documents analyzed, <artifact_documents_reused> artifact documents reused, <modules> modules, <summaries> summaries, <conflicts> conflicts
```

Do not change compatibility output.

### Step 7: Protect backups and errors

Add `program.json` to `BACKUP_ARTIFACTS`. Extend the backup test with distinct
bytes and assert byte-identical recovery.

Add transparent `CoreError` variants for IR validation, provider decoding,
merge, and analysis. Do not convert typed failures to generic strings before
the CLI boundary.

Run:

```bash
cargo test -p compass-core --test program_pipeline
cargo test -p compass-core
cargo test -p compass-cli --test program_cli
cargo test -p compass-output backup
cargo clippy -p compass-core -p compass-cli -p compass-output --all-targets -- -D warnings
```

Expected: all tests pass.

### Step 8: Commit

```bash
git add crates/compass-core crates/compass-cli crates/compass-output
git commit -m "feat(core): build Program IR from evidence providers"
```

## Task 8: Preserve Program IR in immutable history

**Files:**

- Modify: `crates/compass-history/Cargo.toml`
- Modify: `crates/compass-history/src/artifacts.rs`
- Modify: `crates/compass-history/src/fingerprint.rs`
- Modify: `crates/compass-history/src/model.rs`
- Modify: `crates/compass-history/src/store.rs`
- Modify: `crates/compass-history/src/diff.rs`
- Modify: `crates/compass-history/src/gc.rs`
- Modify: `crates/compass-history/src/validate.rs`
- Modify: `crates/compass-history/tests/roundtrip.rs`
- Modify: `crates/compass-history/tests/diff.rs`
- Modify: `crates/compass-history/tests/maintenance.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

### Step 1: Write schema-3 compatibility tests

Bump `HISTORY_SCHEMA_VERSION` from 2 to 3 and add tests that:

- ingest schema-2 realizations with empty program trees;
- store and reopen Program IR facts and summaries;
- preserve providers and evidence exactly;
- change realization identity when provider input digest changes even if the
  structural graph is unchanged;
- exclude an absolute artifact path from identity;
- share unchanged program subtrees between revisions;
- include program facts in full diff and exclude them from topology-only diff;
- make GC retain reachable program chunks and delete unreachable ones;
- export canonical `program.json` in `history export --format compass-out`.

Run:

```bash
cargo test -p compass-history
```

Expected: schema and field assertions fail.

### Step 2: Add program trees to the realization

Add:

```rust
pub program_facts_root: StoredTree,
pub program_summaries_root: StoredTree,
pub program_fact_count: u64,
pub program_summary_count: u64,
```

Add a schema-aware deserializer that supplies empty roots/counts using the
current Prolly tree format for schema-2 input; do not rely on a derived default
whose tree format could drift. The version reader explicitly accepts versions 2
and 3. New writes use schema 3 only.
Partition canonical Program IR by stable logical keys:

```text
provider/<provider-id>
evidence/<evidence-id>
module/<source-file>
summary/<symbol-id>
reverse-call/<target-symbol-id>
```

Do not store the entire JSON sidecar as one opaque value.

### Step 3: Extend profile and extraction fingerprint

The build profile records enabled provider policy, IR schema, merger version,
analysis schema, and analyzer version. The extraction fingerprint records the
actual canonical provider manifest, including artifact content digests and
configuration digests.

Do not include artifact filesystem paths. A change from raw unverified SCIP to
the same SCIP plus a freshness manifest changes the configuration digest and
therefore realization identity.

### Step 4: Extend diff, GC, and export

Full diff reports added, removed, and changed program keys plus summary counts.
Topology-only diff deliberately ignores program roots. GC traverses both new
trees. Export reconstructs `AnalysisBundle`, validates it, obtains canonical
bytes, and writes exactly `program.json`.

Run:

```bash
cargo test -p compass-history
cargo test -p compass-cli history
cargo clippy -p compass-history --all-targets -- -D warnings
```

Expected: schema-2 compatibility and schema-3 round trips pass.

### Step 5: Commit

```bash
git add crates/compass-history crates/compass-cli/src/history_commands.rs crates/compass-cli/tests/history_cli.rs
git commit -m "feat(history): version Program IR evidence"
```

## Task 9: Qualify determinism, scale, security, and documentation

**Files:**

- Create: `scripts/qualify_program_ir.sh`
- Create: `fixtures/program-ir/README.md`
- Create: `fixtures/program-ir/rust/`
- Create: `fixtures/program-ir/typescript/`
- Create: `fixtures/program-ir/scip/`
- Modify: `README.md`
- Modify: `docs/roadmap.md`
- Modify: `docs/reference/commands.md`
- Modify: `docs/reference/outputs.md`
- Modify: `docs/design/architecture.md`
- Modify: `docs/design/storage-and-history.md`
- Modify: `docs/design/security-and-privacy.md`

Only modify a listed documentation file if it is tracked when implementation
begins. If the documentation-system work is not yet merged, update `README.md`
and this plan's roadmap design instead of inventing a parallel documentation
tree.

### Step 1: Add the qualification corpus

Fixtures cover:

- Rust traits, impl collisions, macros, `?`, async, and ambiguous calls;
- TypeScript imports, overload-like syntax, decorators, JSX, callbacks,
  dynamic properties, and async;
- matching, stale, conflicting, UTF-16, malformed, and oversized SCIP;
- same-content/different-path and same-repository/different-checkout cases.

Each fixture includes expected provider coverage and expected conflicts, not
only output snapshots.

### Step 2: Add the qualification script

The script must:

1. build Compass once;
2. run every new package test;
3. perform cold, warm, syntax-change, artifact-change, and clean rebuilds;
4. compare canonical `program.json` bytes after each equivalent state;
5. assert an artifact-only change reports zero syntax analyses;
6. run two checkout roots and compare bytes;
7. run history ingest/reopen/diff/GC/export;
8. assert Graphify compatibility produces no `program.json`;
9. run workspace format, tests, and denied Clippy.

Use a temporary directory from `mktemp -d` and a trap for cleanup. Do not
modify tracked fixtures during the run.

### Step 3: Document honest capability boundaries

Document:

- what Program IR can support now and later;
- why Tree-sitter is a baseline rather than compiler semantics;
- how official SCIP is supplied and how freshness is proven;
- the exact meaning of capability coverage and conflicts;
- no indexer or language server is invoked;
- the difference between structural SCIP JSON ingestion and official protobuf
  Program IR enrichment;
- `program.json` schema and history behavior;
- native versus Graphify compatibility behavior.

Remove or correct any statement claiming full semantic resolution from
Tree-sitter alone.

### Step 4: Run complete verification

Run:

```bash
bash scripts/qualify_program_ir.sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
compass update .
cd /Users/haipingfu/graphify && graphify update .
```

Expected: every command succeeds, clean and incremental Program IR bytes match,
and both repository graphs are current.

### Step 5: Commit

```bash
git add scripts/qualify_program_ir.sh fixtures/program-ir README.md docs/roadmap.md docs/reference/commands.md docs/reference/outputs.md docs/design/architecture.md docs/design/storage-and-history.md docs/design/security-and-privacy.md compass-out
git commit -m "test(program): qualify the evidence foundation"
```

## Completion criteria

- `program.json` is the only Program IR artifact name.
- Tree-sitter is represented and tested as a syntax provider, not a universal
  semantic generator.
- Official SCIP is decoded from protobuf without invoking external tools.
- Raw SCIP freshness uncertainty is visible; the optional companion manifest
  can prove or disprove document freshness.
- File, artifact, and project provider scopes are explicit and independently
  cacheable.
- Every semantic fact can be traced to registered evidence.
- Coverage is per capability and conflicts are preserved.
- Provider and input order cannot affect canonical bytes.
- Artifact-only changes reuse syntax evidence.
- Unsupported languages do not receive fabricated Program IR bodies.
- Native update/extract/watch emit and report `program.json`.
- Graphify compatibility output is unchanged.
- Failed provider or output phases do not replace the previous valid artifact.
- History schema 3 reads schema 2 and versions Program IR in separate Prolly
  roots.
- Full diff, topology-only diff, GC, export, backup, and unchanged fast paths
  handle Program IR correctly.
- Clean and incremental builds are byte-equivalent across checkout roots.
- Workspace tests and denied Clippy pass.
- Compass and superproject knowledge graphs are refreshed after code changes.

## Follow-on plans

Write separate design and implementation plans, in this order:

1. TypeScript compiler project analyzer and build-context fingerprinting.
2. Rust call resolution using a stable rust-analyzer/SCIP path; evaluate rustc
   HIR/MIR only behind an explicitly versioned toolchain boundary.
3. Branch-complete CFG and exception/async control flow.
4. Go SSA, Roslyn, and Clang project analyzers.
5. Interprocedural data flow, effects, contracts, and witness paths.
6. Offline runtime/test/coverage/profile overlays.
7. Semantic change, impact prediction, and test selection.
8. Cross-repository contract federation.
9. Read-only LSP and external-system connectors.
10. Graph-grounded agent planning and verification.
