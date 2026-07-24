# Actionable Semantic Diff MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Compass's raw history diff with a deterministic, actionable semantic review that reports likely breaks, behavior changes, affected callers/modules, dependency changes, and evidence-gated test gaps for Python, Rust, and TypeScript/JavaScript.

**Architecture:** Stage 0 makes graph identities independent of detached-worktree paths. Stage 1 enriches Program IR with callable contracts, obtains zero-context Git hunks and targeted history records, and introduces a `compass-semantic-diff` crate that correlates source, graph, Program IR, summaries, and reverse-call evidence into a versioned report. The CLI then hard-cuts `compass diff` over to that report and deletes the legacy raw-change flags, schema, and renderers; the Prolly-tree diff remains an internal history primitive.

**Tech Stack:** Rust 2024 workspace; tree-sitter language providers; Git CLI; `compass-history` Prolly records; `compass-ir`; `compass-analysis`; `serde`/`serde_json`; `sha2`; `thiserror`; Cargo integration tests.

## Global Constraints

- This is a hard cutover: do not retain aliases, compatibility parsing, deprecation warnings, or legacy JSON fields.
- Public command for this plan: `compass diff OLD NEW [--format text|json] [--all] [--explain FINDING_ID] [--fingerprint SHA256]`.
- `--summarize` and hosted PR delivery are Stage 3 and are not advertised or accepted by this MVP.
- Public JSON schema is exactly `compass.semantic_diff.report/1`.
- Raw Prolly record changes remain available to Rust internals but are not rendered by the CLI.
- Finding IDs, ordering, evidence references, and JSON serialization must be deterministic across machines and detached worktrees.
- Exact callable-contract classifiers in this MVP cover Python, Rust, TypeScript, TSX, and JavaScript.
- Conclusions are evidence-gated: incomplete call resolution or test mapping produces `unknown`/`partial`, never an unsupported `safe` or `gap`.
- Default text is actionable-first; routine symbol churn is collapsed unless `--all` is present.
- Resource limits are fixed: 10,000 direct entities, 200,000 traversed call edges, depth 4, 5,000 findings, 20 evidence items per finding.
- Existing uncommitted multigraph work in `crates/compass-graph/src/lib.rs` and `crates/compass-graph/tests/build_coverage.rs` is user-owned and must not be staged by any task.
- After each code-changing task, run `graphify update .` from the Compass repository root as required by `AGENTS.md`.
- Keep the currently untracked `graphify-out/` refresh local; do not stage it unless repository policy changes and it becomes tracked.
- No history-store migration is provided. `PROGRAM_SCHEMA_VERSION = 2` changes the extraction fingerprint, so users rebuild both compared revisions with the new binary.

---

## File and Module Map

### Existing files modified

- `Cargo.toml`: register `crates/compass-semantic-diff` and its workspace dependency.
- `crates/compass-core/src/pipeline.rs`: canonicalize template edge target identities using logical source paths.
- `crates/compass-core/src/pipeline.rs` tests: prove two physical checkouts produce identical graph identities.
- `crates/compass-ir/src/model.rs`: define callable-contract vocabulary on functions and parameters.
- `crates/compass-ir/src/lib.rs`: bump `PROGRAM_SCHEMA_VERSION` to `2` and re-export new types.
- `crates/compass-ir/tests/canonical.rs`: pin canonical v2 serialization.
- `crates/compass-languages/src/program/mod.rs`: dispatch Python and bump the tree-sitter provider version.
- `crates/compass-languages/src/program/rust.rs`: extract Rust visibility, execution mode, receiver/kind, and requiredness.
- `crates/compass-languages/src/program/typescript.rs`: extract JS/TS visibility, async/generator mode, parameter kind, optionality, and defaults.
- `crates/compass-languages/tests/program_evidence.rs`: assert exact Rust/TS/Python contracts.
- `crates/compass-history/src/git.rs`: expose deterministic file status and zero-context hunks.
- `crates/compass-history/src/store.rs`: expose bounded, typed record lookup by realization and key.
- `crates/compass-history/src/lib.rs`: re-export the new Git and record-read APIs.
- `crates/compass-history/tests/git_repository.rs`: cover adds, deletes, renames, and hunk parsing.
- `crates/compass-history/tests/history_store.rs`: prove present/missing/type-mismatch record reads.
- `crates/compass-cli/Cargo.toml`: depend on `compass-semantic-diff`.
- `crates/compass-cli/src/lib.rs`: route `diff` to its dedicated command module and remove streaming raw-diff entry points.
- `crates/compass-cli/src/bin/compass.rs`: remove the raw-diff streaming special case and use normal outcome writing.
- `crates/compass-cli/src/help.rs`: publish only the replacement command contract.
- `crates/compass-cli/src/history_commands.rs`: retain history management and comparable-realization resolution; delete raw diff parsing/rendering.
- `crates/compass-cli/tests/history_cli.rs`: replace legacy assertions with semantic text, JSON, explain, filtering, and rejection tests.
- `README.md`, `docs/reference/commands.md`, `docs/guides/versioned-history.md`, `docs/cookbook/impact-analysis.md`, `docs/cookbook/ci-and-automation.md`, `docs/roadmap.md`, `crates/compass-cli/assets/compass-skill/references/history.md`: replace raw-diff guidance with semantic review guidance.

### New files

- `crates/compass-languages/src/program/python.rs`: Python Program IR extraction.
- `crates/compass-semantic-diff/Cargo.toml`: semantic-diff crate manifest.
- `crates/compass-semantic-diff/src/lib.rs`: public API and orchestration.
- `crates/compass-semantic-diff/src/error.rs`: bounded loading, invalid comparison, rendering, and serialization errors.
- `crates/compass-semantic-diff/src/model.rs`: versioned report, finding, evidence, impact, and completeness types.
- `crates/compass-semantic-diff/src/input.rs`: bounded adapter contracts and entity alignment.
- `crates/compass-semantic-diff/src/contracts.rs`: language-neutral classifier and language-specific compatibility rules.
- `crates/compass-semantic-diff/src/behavior.rs`: summary/effect/error/dependency comparison.
- `crates/compass-semantic-diff/src/impact.rs`: bounded reverse-call traversal and affected-module aggregation.
- `crates/compass-semantic-diff/src/verification.rs`: evidence-gated static test mapping.
- `crates/compass-semantic-diff/src/rank.rs`: reviewer-priority ranking and routine-churn collapse.
- `crates/compass-semantic-diff/tests/semantic_diff.rs`: focused pipeline fixtures.
- `crates/compass-cli/src/semantic_diff_commands.rs`: hard-cutover CLI parser, resolver adapter, and render dispatch.
- `crates/compass-cli/src/semantic_diff_render.rs`: deterministic text, canonical JSON, and explain rendering.
- `crates/compass-cli/tests/golden/actionable.txt`: stable default text.
- `crates/compass-cli/tests/golden/actionable.json`: stable schema-v1 JSON.

---

### Task 1: Make Template Identities Independent of Checkout Paths

**Files:**
- Modify: `crates/compass-languages/src/templates.rs`
- Modify: `crates/compass-core/src/pipeline.rs`

**Interfaces:**
- Consumes: existing `finalize_ast_extraction(extraction: &mut Extraction, root: &Path, ast_id_remap: &HashMap<String, String>)`.
- Produces: `resolve_template_import(path: &Path, dynamic: bool) -> PathBuf`, which resolves an existing relative template import to its real source extension before the existing AST identity remap runs.

- [ ] **Step 1: Write the cross-checkout regression test**

In the existing `pipeline.rs` test module, write the same `src/Page.astro` and `src/Layout.astro` under two `tempfile::TempDir` roots, run `build_graph_with_layers` with `BuildOptions::new(root)` and `no_viz = true`, sort `(node.id, edge.source, edge.target, edge.relation)` tuples, and assert equality plus absence of both absolute root strings:

```rust
#[test]
fn astro_import_identities_do_not_include_checkout_root() -> Result<(), Box<dyn Error>> {
    let left = astro_fixture()?;
    let right = astro_fixture()?;
    let left_graph = build_fixture_graph(left.path())?;
    let right_graph = build_fixture_graph(right.path())?;

    assert_eq!(identity_rows(&left_graph), identity_rows(&right_graph));
    let encoded = serde_json::to_string(&left_graph).expect("json");
    assert!(!encoded.contains(left.path().to_string_lossy().as_ref()));
    assert!(!encoded.contains(right.path().to_string_lossy().as_ref()));
    Ok(())
}
```

- [ ] **Step 2: Run the regression test and verify the reproduced failure**

Run: `cargo test -p compass-core pipeline::tests::astro_import_identities_do_not_include_checkout_root -- --exact`

Expected: FAIL because the Astro `IMPORTS` target contains a `git_compass_tmp_worktree_*`/temporary-root-derived ID.

- [ ] **Step 3: Resolve template imports to real source files**

Replace the static/dynamic split with one existing-file resolver:

```rust
fn resolve_template_import(path: &Path, dynamic: bool) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    let rewritten = rewrite_js_extension(path.to_path_buf());
    if rewritten.is_file() {
        return rewritten;
    }
    for extension in [
        "astro", "svelte", "vue", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs",
    ] {
        let candidate = path.with_extension(extension);
        if candidate.is_file() {
            return candidate;
        }
    }
    if dynamic {
        resolve_js_path(path)
    } else {
        rewritten
    }
}
```

Call this helper for all relative template imports. When the file exists, its absolute `source_file` now matches `live_sources`, so the existing `collect_ast_id_remap` converts both the placeholder node and `IMPORTS` edge target to the root-relative file ID. Keep unresolved project references unchanged; never invent an absolute fallback for an existing file.

- [ ] **Step 4: Verify identities and existing extraction coverage**

Run:

```bash
cargo test -p compass-core pipeline::tests::astro_import_identities_do_not_include_checkout_root -- --exact
cargo test -p compass-languages --test engine_edge_coverage
graphify update .
```

Expected: all tests PASS; graph update exits `0`; serialized output contains only root-relative source paths.

- [ ] **Step 5: Commit the identity fix**

```bash
git add crates/compass-languages/src/templates.rs crates/compass-core/src/pipeline.rs
git commit -m "fix: stabilize template identities across revisions"
```

---

### Task 2: Define Program IR Callable Contracts v2

**Files:**
- Modify: `crates/compass-ir/src/model.rs`
- Modify: `crates/compass-ir/src/lib.rs`
- Modify: `crates/compass-ir/tests/canonical.rs`

**Interfaces:**
- Produces:
  - `Visibility::{Public, Protected, Internal, Private, Unknown}`
  - `ExecutionMode::{Sync, Async, Generator, AsyncGenerator, Unknown}`
  - `ParameterKind::{Receiver, PositionalOnly, PositionalOrKeyword, KeywordOnly, VariadicPositional, VariadicKeyword}`
  - `FunctionIr.visibility: Visibility`
  - `FunctionIr.execution_mode: ExecutionMode`
  - `ParameterIr.kind: ParameterKind`
  - `ParameterIr.required: bool`
  - `ParameterIr.default_digest: Option<String>`
  - `PROGRAM_SCHEMA_VERSION: u32 = 2`

- [ ] **Step 1: Write failing canonical-v2 tests**

Add a serialization test with a public async function, a required positional parameter, and an optional keyword-only parameter. Assert exact snake-case values and the default digest:

```rust
assert_eq!(value["visibility"], "public");
assert_eq!(value["execution_mode"], "async");
assert_eq!(value["parameters"][0]["kind"], "positional_or_keyword");
assert_eq!(value["parameters"][0]["required"], true);
assert_eq!(value["parameters"][1]["kind"], "keyword_only");
assert_eq!(value["parameters"][1]["default_digest"], "sha256:default");
assert_eq!(PROGRAM_SCHEMA_VERSION, 2);
```

- [ ] **Step 2: Verify the new vocabulary is absent**

Run: `cargo test -p compass-ir canonical_function_contract_v2 -- --exact`

Expected: compilation FAILS because the enum types and fields do not exist.

- [ ] **Step 3: Add the v2 types and required fields**

Add these exact enum definitions with `#[serde(rename_all = "snake_case")]`:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Protected,
    Internal,
    Private,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Sync,
    Async,
    Generator,
    AsyncGenerator,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    Receiver,
    PositionalOnly,
    PositionalOrKeyword,
    KeywordOnly,
    VariadicPositional,
    VariadicKeyword,
}
```

Add the fields listed in **Produces** without serde defaults. This intentionally makes v1 artifacts fail to deserialize under v2. Set `PROGRAM_SCHEMA_VERSION` to `2` and update every in-repository fixture/constructor until the crate compiles; use `Unknown`, `Sync`, `PositionalOrKeyword`, `required: true`, and `default_digest: None` only where a provider cannot yet supply stronger evidence.

- [ ] **Step 4: Verify canonical serialization and the workspace consumers**

Run:

```bash
cargo test -p compass-ir
cargo check --workspace
graphify update .
```

Expected: all tests PASS; workspace check exits `0`; canonical JSON contains no compatibility-only omitted fields.

- [ ] **Step 5: Commit the Program IR schema cutover**

```bash
git add crates/compass-ir crates/compass-analysis crates/compass-history crates/compass-languages crates/compass-program crates/compass-cli
git commit -m "feat: define callable contracts in program IR v2"
```

---

### Task 3: Populate Rust and TypeScript/JavaScript Contracts

**Files:**
- Modify: `crates/compass-languages/src/program/rust.rs`
- Modify: `crates/compass-languages/src/program/typescript.rs`
- Modify: `crates/compass-languages/src/program/mod.rs`
- Modify: `crates/compass-languages/tests/program_evidence.rs`

**Interfaces:**
- Consumes: v2 contract types from Task 2.
- Produces: exact contract evidence for Rust, TypeScript, TSX, and JavaScript; `TREE_SITTER_PROGRAM_PROVIDER_VERSION = 3`.

- [ ] **Step 1: Add failing Rust and TypeScript fixture assertions**

Use fixtures containing `pub async fn fetch(&self, id: u64, limit: Option<u32>)` and `export async function load(id: string, limit = 20, ...tags: string[])`. Assert:

```rust
assert_eq!(rust.visibility, Visibility::Public);
assert_eq!(rust.execution_mode, ExecutionMode::Async);
assert_eq!(rust.parameters[0].kind, ParameterKind::Receiver);
assert!(rust.parameters[1].required);

assert_eq!(ts.visibility, Visibility::Public);
assert_eq!(ts.execution_mode, ExecutionMode::Async);
assert!(ts.parameters[0].required);
assert!(!ts.parameters[1].required);
assert!(ts.parameters[1].default_digest.is_some());
assert_eq!(ts.parameters[2].kind, ParameterKind::VariadicPositional);
```

- [ ] **Step 2: Run provider tests and verify incorrect defaults**

Run: `cargo test -p compass-languages --test program_evidence callable_contracts -- --exact`

Expected: FAIL because the providers still emit Task 2 fallback values.

- [ ] **Step 3: Extract exact syntax facts**

Implement focused helpers:

```rust
fn rust_visibility(function: Node<'_>) -> Visibility;
fn rust_execution_mode(function: Node<'_>) -> ExecutionMode;
fn rust_parameter(input: &FileInput<'_>, node: Node<'_>, evidence_id: &str) -> ParameterIr;

fn js_visibility(function: Node<'_>) -> Visibility;
fn js_execution_mode(function: Node<'_>) -> ExecutionMode;
fn js_parameter(input: &FileInput<'_>, node: Node<'_>, evidence_id: &str) -> ParameterIr;
```

For defaults, store `Some(hex_sha256(slice(input.source, default_node)))`; never store source text. Rust parameters are required unless syntactically variadic; `self`/`&self`/`&mut self` are receivers. JS/TS `?` and initializer parameters are optional; rest parameters are `VariadicPositional`. Exported declarations are public, `private`/`protected` members retain that visibility, and other declarations are internal.

- [ ] **Step 4: Verify provider facts and fingerprints**

Run:

```bash
cargo test -p compass-languages --test program_evidence
cargo test -p compass-program
graphify update .
```

Expected: all tests PASS; provider descriptors contain `tree-sitter/3`.

- [ ] **Step 5: Commit provider enrichment**

```bash
git add crates/compass-languages
git commit -m "feat: extract rust and typescript callable contracts"
```

---

### Task 4: Add the Python Program IR Provider

**Files:**
- Create: `crates/compass-languages/src/program/python.rs`
- Modify: `crates/compass-languages/src/program/mod.rs`
- Modify: `crates/compass-languages/tests/program_evidence.rs`

**Interfaces:**
- Consumes: `EvidenceBatch`, `FunctionIr`, and v2 contract types.
- Produces: `python::extract(descriptor: ProviderDescriptor, input: &FileInput<'_>, root: Node<'_>) -> EvidenceBatch`.

- [ ] **Step 1: Add a failing Python extraction test**

Use:

```python
async def fetch(self, account_id: str, /, limit: int = 20, *, trace: bool, **options) -> Result:
    return client.load(account_id)
```

Assert one function, async mode, `self` receiver, positional-only `account_id`, optional `limit` with digest, required keyword-only `trace`, variadic-keyword `options`, return type `Result`, and a call operation for `client.load`.

- [ ] **Step 2: Verify Python is not dispatched**

Run: `cargo test -p compass-languages --test program_evidence python_callable_contract -- --exact`

Expected: FAIL because `analyze_file` returns `None` for Python.

- [ ] **Step 3: Implement Python extraction and dispatch**

Mirror the existing provider evidence model, not its source layout. Export exactly:

```rust
pub(super) fn extract(
    descriptor: ProviderDescriptor,
    input: &FileInput<'_>,
    root: Node<'_>,
) -> EvidenceBatch;
```

Recognize `function_definition`, extract decorated/nested functions, classify `/`, `*`, `*args`, and `**kwargs`, hash default expressions, collect call/return/read/write operations using existing `OperationKind`, set `_name` module functions private and other module functions public, and mark unresolved capabilities `Partial` with explicit reason strings.

Update dispatch:

```rust
mod python;

if spec.kind != ExtractorKind::Generic
    || !matches!(spec.name, "python" | "rust" | "typescript" | "tsx" | "javascript")
{
    return Ok(None);
}
let batch = match spec.name {
    "python" => python::extract(descriptor, &normalized, tree.root_node()),
    "rust" => rust::extract(descriptor, &normalized, tree.root_node()),
    "typescript" | "tsx" | "javascript" => {
        typescript::extract(descriptor, &normalized, tree.root_node())
    }
    _ => return Ok(None),
};
```

- [ ] **Step 4: Verify Python plus all language evidence**

Run:

```bash
cargo test -p compass-languages --test program_evidence
cargo test -p compass-analysis
graphify update .
```

Expected: all tests PASS; Python evidence survives `merge_evidence`; incomplete Python call resolution remains marked `Partial`.

- [ ] **Step 5: Commit Python Program IR**

```bash
git add crates/compass-languages
git commit -m "feat: extract python program contracts"
```

---

### Task 5: Expose Deterministic Git Source Deltas

**Files:**
- Modify: `crates/compass-history/src/git.rs`
- Modify: `crates/compass-history/src/lib.rs`
- Modify: `crates/compass-history/tests/git_repository.rs`

**Interfaces:**
- Produces:

```rust
pub enum SourceFileStatus { Added, Modified, Deleted, Renamed }
pub struct SourceHunk { pub old_start: u32, pub old_lines: u32, pub new_start: u32, pub new_lines: u32 }
pub struct SourceFileDelta {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub status: SourceFileStatus,
    pub hunks: Vec<SourceHunk>,
}
impl Repository {
    pub fn source_delta(&self, old: &CommitId, new: &CommitId) -> Result<Vec<SourceFileDelta>, HistoryError>;
}
```

- [ ] **Step 1: Write repository tests for all statuses and zero-context hunks**

Create commits that add, edit, rename, and delete files including a path with spaces. Assert root-relative `/`-separated paths, sorted output, rename pairing, and exact `(old_start, old_lines, new_start, new_lines)` values.

- [ ] **Step 2: Verify the API is absent**

Run: `cargo test -p compass-history --test git_repository source_delta -- --exact`

Expected: compilation FAILS because `Repository::source_delta` and its types do not exist.

- [ ] **Step 3: Implement NUL-safe status and hunk parsing**

Run Git without fetching or checking out:

```text
git diff --raw -z --find-renames=50% OLD NEW --
git diff --no-ext-diff --no-textconv --find-renames=50% --unified=0 --no-color OLD NEW --
```

Parse `--raw -z` for authoritative path/status identity. Treat each `diff --git` header in the patch stream only as an ordinal file separator—never recover identity from its text—then attach parsed `@@ -a,b +c,d @@` headers to the matching raw-status entry in Git's output order. Reject count/order mismatch, malformed or duplicate entries, and absolute or parent-traversing raw paths with `HistoryError::Git`; sort the completed records by `(new_path, old_path, status)`.

- [ ] **Step 4: Verify source delta behavior**

Run:

```bash
cargo test -p compass-history --test git_repository
cargo test -p compass-history
graphify update .
```

Expected: all tests PASS, including spaces, binary files with no hunks, and rename-only files.

- [ ] **Step 5: Commit source deltas**

```bash
git add crates/compass-history
git commit -m "feat: expose deterministic git source deltas"
```

---

### Task 6: Add Bounded Typed History Record Reads

**Files:**
- Modify: `crates/compass-history/src/artifacts.rs`
- Modify: `crates/compass-history/src/store.rs`
- Modify: `crates/compass-history/src/lib.rs`
- Modify: `crates/compass-history/tests/history_store.rs`

**Interfaces:**
- Produces:

```rust
pub enum HistoryRecordKey<'a> {
    Node(&'a str),
    ProgramModule(&'a str),
    ProgramSummary(&'a str),
    ReverseCallers(&'a str),
}

pub enum HistoryRecord {
    Node(compass_model::NodeRecord),
    ProgramModule(compass_ir::ModuleIr),
    ProgramSummary(compass_analysis::FunctionSummary),
    ReverseCallers(Vec<String>),
}

impl HistoryStore {
    pub fn read_record(
        &self,
        realization: &RealizationId,
        key: HistoryRecordKey<'_>,
    ) -> Result<Option<HistoryRecord>, HistoryError>;
}
```

- [ ] **Step 1: Write present, absent, and malformed-record tests**

Publish a fixture containing one node, module, summary, and reverse-call record. Assert typed values, `Ok(None)` for a valid missing key, and `HistoryError::CorruptHistory` when a value under a known typed key has the wrong JSON shape.

- [ ] **Step 2: Verify targeted reads are unavailable**

Run: `cargo test -p compass-history --test history_store reads_typed_history_records -- --exact`

Expected: compilation FAILS because `HistoryRecordKey` and `read_record` do not exist.

- [ ] **Step 3: Implement one-key Prolly reads**

Make `program_key` `pub(crate)`, map keys through existing `node_key`/`program_key` helpers, select the correct stored tree from the realization manifest, enforce `MAX_RECORD_VALUE_BYTES`, and deserialize exactly one value:

```rust
let Some(bytes) = self.prolly.get(&tree, &encoded_key)? else {
    return Ok(None);
};
if bytes.len() > MAX_RECORD_VALUE_BYTES {
    return Err(HistoryError::CorruptHistory(
        "history record exceeds byte limit".to_owned(),
    ));
}
decode_history_record(key, &bytes).map(Some)
```

Do not call `HistoryStore::artifacts`; do not scan a tree.

- [ ] **Step 4: Verify record reads and existing store behavior**

Run:

```bash
cargo test -p compass-history --test history_store
cargo test -p compass-history
graphify update .
```

Expected: all tests PASS and the targeted-read test observes one Prolly lookup per request.

- [ ] **Step 5: Commit typed record access**

```bash
git add crates/compass-history
git commit -m "feat: read targeted semantic history records"
```

---

### Task 7: Create the Versioned Semantic Report Model

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/compass-semantic-diff/Cargo.toml`
- Create: `crates/compass-semantic-diff/src/lib.rs`
- Create: `crates/compass-semantic-diff/src/error.rs`
- Create: `crates/compass-semantic-diff/src/model.rs`
- Create: `crates/compass-semantic-diff/tests/semantic_diff.rs`

**Interfaces:**
- Produces:

```rust
pub const REPORT_SCHEMA: &str = "compass.semantic_diff.report/1";
pub const MAX_DIRECT_ENTITIES: usize = 10_000;
pub const MAX_TRAVERSED_CALL_EDGES: usize = 200_000;
pub const MAX_IMPACT_DEPTH: u8 = 4;
pub const MAX_FINDINGS: usize = 5_000;
pub const MAX_EVIDENCE_PER_FINDING: usize = 20;

pub enum FindingType { ContractChange, BehaviorChange, DependencyChange, ImpactChange, VerificationGap, StructuralChange }
pub enum FindingOrigin { Direct, Derived }
pub enum Compatibility { ProvenBreak, PossibleBreak, Compatible, Behavioral, NotApplicable, Indeterminate }
pub enum Confidence { Exact, Probable, Inferred, Unknown }
pub enum Completeness { Complete, Partial, Unavailable }
pub enum VerificationState { Unknown, Partial, Covered, Gap, Stale, Failing, NotRun }
pub enum ChangeDirection { Added, Removed }
pub struct EvidenceRef { pub source_file: String, pub start_byte: Option<u64>, pub end_byte: Option<u64>, pub record_key: Option<String>, pub capability: String }
pub struct AffectedConsumer { pub symbol_id: String, pub display_name: String, pub source_file: String, pub distance: u8 }
pub struct WitnessHop { pub source: String, pub relation: String, pub target: String, pub confidence: Confidence }
pub struct WitnessPath { pub consumer: String, pub confidence: Confidence, pub hops: Vec<WitnessHop> }
pub struct Verification { pub state: VerificationState, pub exact_tests: Vec<String>, pub recommended_tests: Vec<String>, pub reason: String }
pub struct TestEvidence { pub completeness: Completeness, pub exact_tests: Vec<String>, pub suggested_tests: Vec<String> }
pub trait TestEvidenceProvider { fn tests_for(&self, symbol_id: &str) -> TestEvidence; }
pub struct SemanticFinding {
    pub id: String,
    pub finding_type: FindingType,
    pub subject: String,
    pub origin: FindingOrigin,
    pub headline: String,
    pub explanation: String,
    pub compatibility: Compatibility,
    pub confidence: Confidence,
    pub review_priority: u16,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
    pub affected_consumers: Vec<AffectedConsumer>,
    pub witness_paths: Vec<WitnessPath>,
    pub verification: Verification,
    pub reviewer_action: String,
    pub evidence: Vec<EvidenceRef>,
    pub completeness: BTreeMap<String, Completeness>,
}
pub struct CollapsedGroup { pub label: String, pub count: usize, pub finding_ids: Vec<String> }
pub struct Comparison { pub old_commit: String, pub new_commit: String, pub fingerprint: String }
pub struct SemanticDiffReport {
    pub schema: String,
    pub comparison: Comparison,
    pub findings: Vec<SemanticFinding>,
    pub collapsed_groups: Vec<CollapsedGroup>,
    pub completeness: BTreeMap<String, Completeness>,
    pub limitations: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SemanticDiffError {
    #[error("semantic diff input is invalid: {0}")]
    InvalidInput(String),
    #[error("semantic evidence could not be read: {0}")]
    Evidence(String),
    #[error("semantic finding {0} does not exist")]
    FindingNotFound(String),
    #[error("semantic diff {resource} limit exceeded ({limit})")]
    LimitExceeded { resource: &'static str, limit: usize },
    #[error("semantic report serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}
```

- [ ] **Step 1: Write failing model and stable-ID tests**

Construct the same finding with evidence inserted in opposite orders. Assert identical canonical JSON and identical ID matching `sd1-` plus 24 lowercase hex characters. Move source line/byte anchors and assert the ID does not change. Assert exceeding evidence capacity returns `SemanticDiffError::LimitExceeded { resource: "evidence_per_finding", limit: 20 }`.

- [ ] **Step 2: Verify the crate is absent**

Run: `cargo test -p compass-semantic-diff`

Expected: FAIL because the package does not exist.

- [ ] **Step 3: Add the crate and canonical model**

Register the member and dependencies. Implement:

```rust
pub fn finding_id(
    finding_type: FindingType,
    subject: &str,
    before: Option<&serde_json::Value>,
    after: Option<&serde_json::Value>,
    classifier_version: u32,
    relationship_identities: &[String],
) -> String;

pub fn finalize_report(
    report: SemanticDiffReport,
) -> Result<SemanticDiffReport, SemanticDiffError>;
```

Canonicalize paths and evidence, hash `(report schema, classifier version, subject, finding type, before, after, sorted relationship identities)` with SHA-256, use the first 12 bytes as the ID suffix, sort/deduplicate every vector, enforce all constants by returning `LimitExceeded`, and set `schema` internally rather than accepting it from callers. Exclude commits, line/byte anchors, timestamps, display text, and model prose from the ID.

- [ ] **Step 4: Verify the model**

Run:

```bash
cargo test -p compass-semantic-diff
graphify update .
```

Expected: all tests PASS; repeated runs emit byte-identical JSON.

- [ ] **Step 5: Commit the semantic report foundation**

```bash
git add Cargo.toml Cargo.lock crates/compass-semantic-diff
git commit -m "feat: add semantic diff report model"
```

---

### Task 8: Load and Align Directly Changed Entities

**Files:**
- Create: `crates/compass-semantic-diff/src/input.rs`
- Modify: `crates/compass-semantic-diff/src/lib.rs`
- Modify: `crates/compass-semantic-diff/tests/semantic_diff.rs`

**Interfaces:**
- Consumes: bounded source deltas and snapshot adapters assembled outside the semantic engine.
- Produces:

```rust
pub enum SnapshotSide { Old, New }

pub struct SnapshotIdentity {
    pub commit: String,
    pub realization: String,
    pub fingerprint: String,
}

pub trait SnapshotReader {
    fn node(
        &self,
        side: SnapshotSide,
        node_id: &str,
    ) -> Result<Option<NodeRecord>, SemanticDiffError>;
    fn module(
        &self,
        side: SnapshotSide,
        source_file: &str,
    ) -> Result<Option<ModuleIr>, SemanticDiffError>;
    fn summary(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Option<FunctionSummary>, SemanticDiffError>;
    fn reverse_callers(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Vec<String>, SemanticDiffError>;
}

pub struct DependencyDelta {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub change: ChangeDirection,
    pub evidence: Vec<EvidenceRef>,
}

pub struct SemanticDiffInput<'a> {
    pub old: SnapshotIdentity,
    pub new: SnapshotIdentity,
    pub source_deltas: &'a [SourceFileDelta],
    pub changed_node_ids: &'a [String],
    pub dependency_deltas: &'a [DependencyDelta],
    pub snapshots: &'a dyn SnapshotReader,
    pub test_evidence: &'a dyn TestEvidenceProvider,
}

pub struct EntitySnapshot {
    pub language: String,
    pub source_file: String,
    pub function: FunctionIr,
}

pub struct ChangedEntity {
    pub old: Option<EntitySnapshot>,
    pub new: Option<EntitySnapshot>,
    pub hunks: Vec<SourceHunk>,
}

pub fn compare(input: SemanticDiffInput<'_>) -> Result<SemanticDiffReport, SemanticDiffError>;
```

- [ ] **Step 1: Add alignment tests**

Cover: unchanged function outside hunks is ignored; stable symbol ID aligns directly; moved function aligns by `(language, normalized path, qualified name, signature shape)`; ambiguous candidates produce an `unknown` limitation; an absolute/temp-worktree identity is excluded with its source recorded as a limitation; more than 10,000 candidates returns `LimitExceeded { resource: "direct_entities", limit: 10_000 }` without further adapter reads.

- [ ] **Step 2: Verify compare is unavailable**

Run: `cargo test -p compass-semantic-diff aligns_changed_entities -- --exact`

Expected: compilation FAILS because `SemanticDiffRequest` and `compare` do not exist.

- [ ] **Step 3: Implement bounded collection and alignment**

Add:

```rust
fn collect_changed_entities(
    input: &SemanticDiffInput<'_>,
) -> Result<Vec<ChangedEntity>, SemanticDiffError>;

fn align_entity(
    old_candidates: &[FunctionIr],
    new_candidates: &[FunctionIr],
) -> Alignment;
```

Use source hunks as the primary candidate bound. Ask `SnapshotReader::module` only for paths present in `SourceFileDelta`, then select functions whose anchors intersect a hunk. For changed node IDs not represented in Program IR, read only those nodes and preserve any exact contract attributes as incomplete fallback evidence; a signature/body digest by itself becomes an indeterminate implementation finding, never narrated behavior. Direct symbol-ID equality is exact confidence; unique structural fallback is probable confidence; ambiguity is never guessed. Validate that input fingerprints are equal before adapter reads; return `InvalidInput` with an instruction to rebuild from the same profile when they differ. The engine never discovers a repository, invokes Git, opens history, or streams raw records.

- [ ] **Step 4: Verify collection and limits**

Run:

```bash
cargo test -p compass-semantic-diff aligns_changed_entities -- --exact
cargo test -p compass-semantic-diff enforces_direct_entity_limit -- --exact
graphify update .
```

Expected: PASS; the limit fixture returns the exact `LimitExceeded` error and performs no read after entity 10,000.

- [ ] **Step 5: Commit semantic input alignment**

```bash
git add crates/compass-semantic-diff
git commit -m "feat: align changed semantic entities"
```

---

### Task 9: Classify Callable Contract Changes

**Files:**
- Create: `crates/compass-semantic-diff/src/contracts.rs`
- Modify: `crates/compass-semantic-diff/src/lib.rs`
- Modify: `crates/compass-semantic-diff/tests/semantic_diff.rs`

**Interfaces:**
- Consumes: `ChangedEntity`.
- Produces: `classify_contract_change(entity: &ChangedEntity) -> Vec<FindingDraft>`.

- [ ] **Step 1: Add table-driven classifier tests**

Assert these exact outcomes:

```text
removed public callable                          proven_break/exact
required parameter added                        proven_break/exact
parameter removed                               proven_break/exact
parameter kind changed                          proven_break/exact
sync changed to async                           proven_break/exact
return type incompatibly narrowed               proven_break/exact
visibility narrowed                             proven_break/exact
optional parameter added                        compatible/exact
parameter renamed in Python keyword-callable    possible_break/probable
parameter renamed in Rust                       compatible/exact
unavailable type evidence                       indeterminate/unknown
```

Also assert the title names the symbol and the explanation states old/new facts without source-line prose.

- [ ] **Step 2: Verify classification is absent**

Run: `cargo test -p compass-semantic-diff classifies_contract_changes -- --exact`

Expected: compilation FAILS because `classify_contract_change` does not exist.

- [ ] **Step 3: Implement language-specific compatibility rules**

Define:

```rust
pub(crate) struct FindingDraft {
    pub finding_type: FindingType,
    pub origin: FindingOrigin,
    pub compatibility: Compatibility,
    pub confidence: Confidence,
    pub headline: String,
    pub explanation: String,
    pub subject: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
    pub verification: Verification,
    pub reviewer_action: String,
    pub evidence: Vec<EvidenceRef>,
    pub completeness: BTreeMap<String, Completeness>,
}

pub(crate) trait ContractClassifier {
    fn classify(&self, old: &FunctionIr, new: &FunctionIr) -> Vec<FindingDraft>;
}
```

Dispatch by `EntitySnapshot.language`. Use syntax facts only when their corresponding coverage is `Complete`; downgrade confidence and wording otherwise. A digest change alone cannot support a behavior claim; orchestration preserves it as an `indeterminate` implementation finding with explicit missing capabilities.

- [ ] **Step 4: Verify classifiers**

Run:

```bash
cargo test -p compass-semantic-diff classifies_contract_changes -- --exact
cargo test -p compass-semantic-diff digest_only_change_is_indeterminate -- --exact
graphify update .
```

Expected: all classifier cases PASS and finding order is stable.

- [ ] **Step 5: Commit contract classification**

```bash
git add crates/compass-semantic-diff
git commit -m "feat: classify callable contract changes"
```

---

### Task 10: Correlate Behavior and Dependency Changes

**Files:**
- Create: `crates/compass-semantic-diff/src/behavior.rs`
- Modify: `crates/compass-semantic-diff/src/lib.rs`
- Modify: `crates/compass-semantic-diff/tests/semantic_diff.rs`

**Interfaces:**
- Consumes: `FunctionSummary`, graph nodes/edges, source hunks, and aligned entities.
- Produces:

```rust
pub(crate) fn classify_behavior(
    entity: &ChangedEntity,
    old_summary: Option<&FunctionSummary>,
    new_summary: Option<&FunctionSummary>,
) -> Vec<FindingDraft>;

pub(crate) fn classify_dependencies(
    old: &DependencyView,
    new: &DependencyView,
) -> Vec<FindingDraft>;

fn behavior_finding(
    confidence: Confidence,
    headline: &str,
    before: serde_json::Value,
    after: serde_json::Value,
) -> FindingDraft;
```

- [ ] **Step 1: Write behavior/dependency correlation tests**

Cover added/removed effects, newly emitted error, removed resolved call, unresolved-to-resolved call, module dependency added/removed, body-only digest churn, and a temp-root-only dependency identity change. Assert body-only churn produces an `indeterminate` implementation finding with incomplete behavior capabilities; assert temp-root-only churn produces no finding.

- [ ] **Step 2: Verify behavior functions are absent**

Run: `cargo test -p compass-semantic-diff correlates_behavior_and_dependencies -- --exact`

Expected: compilation FAILS because the classifiers do not exist.

- [ ] **Step 3: Implement evidence-gated comparison**

Compare normalized sets, require hunk overlap or a changed summary digest, and emit findings only for explainable deltas:

```rust
let old_errors = old.errors.iter().collect::<BTreeSet<_>>();
let new_errors = new.errors.iter().collect::<BTreeSet<_>>();
if new_errors.difference(&old_errors).next().is_some() {
    drafts.push(behavior_finding(
        Confidence::Exact,
        "new error path",
        serde_json::json!(old.errors),
        serde_json::json!(new.errors),
    ));
}
```

Normalize dependency identities to repository-relative module IDs before set comparison. If effects/calls/errors coverage is partial, preserve the delta but set confidence low and add the exact limitation key to evidence.

- [ ] **Step 4: Verify correlations and false-positive suppression**

Run:

```bash
cargo test -p compass-semantic-diff correlates_behavior_and_dependencies -- --exact
cargo test -p compass-semantic-diff ignores_checkout_path_dependency_churn -- --exact
graphify update .
```

Expected: PASS; temp checkout paths never appear in titles, explanations, IDs, or evidence.

- [ ] **Step 5: Commit behavior classification**

```bash
git add crates/compass-semantic-diff
git commit -m "feat: correlate behavior and dependency changes"
```

---

### Task 11: Traverse Affected Callers and Gate Verification Gaps

**Files:**
- Create: `crates/compass-semantic-diff/src/impact.rs`
- Create: `crates/compass-semantic-diff/src/verification.rs`
- Modify: `crates/compass-semantic-diff/src/lib.rs`
- Modify: `crates/compass-semantic-diff/tests/semantic_diff.rs`

**Interfaces:**
- Consumes: `TestEvidenceProvider`, `TestEvidence`, `SnapshotReader`, and `SemanticDiffInput` from Tasks 7–8.
- Produces:

```rust
pub struct StaticTestEvidence;

pub(crate) fn affected_callers(
    snapshots: &dyn SnapshotReader,
    side: SnapshotSide,
    symbols: &[String],
) -> Result<ImpactResult, SemanticDiffError>;
```

- [ ] **Step 1: Add impact and verification tests**

Build a reverse-call graph with a cycle and five levels. Assert breadth-first distances, deduplication, module aggregation, and exact `LimitExceeded` errors at depth 4 and edge 200,000. Assert:

```text
complete + no matching test      => gap
complete + exact test            => covered
partial + no exact test          => partial, no gap
unknown + no evidence            => unknown, no gap
```

- [ ] **Step 2: Verify traversal and verification are absent**

Run: `cargo test -p compass-semantic-diff computes_bounded_impact_and_test_state -- --exact`

Expected: compilation FAILS because the APIs do not exist.

- [ ] **Step 3: Implement bounded BFS and static mapping**

Use `SnapshotReader::reverse_callers` once per visited symbol, a `VecDeque`, and a visited map storing shortest distance. Retain one shortest `WitnessPath` for every reported consumer group; its confidence is the weakest hop (`exact > probable > inferred > unknown`). Before reading edge 200,001 or descending beyond depth 4, return `SemanticDiffError::LimitExceeded` with resource `impact_edges` or `impact_depth`; do not return a partial report.

`StaticTestEvidence` recognizes test modules/files using exact path/name conventions (`tests/`, `test_*.py`, `*_test.py`, `*.test.ts`, `*.spec.ts`, Rust `#[test]`) and correlates only resolved calls. It returns `Partial` unless every module in scope has complete definitions and call-resolution coverage. Only `Complete` may populate `gaps`.

- [ ] **Step 4: Verify limits and evidence gating**

Run:

```bash
cargo test -p compass-semantic-diff computes_bounded_impact_and_test_state -- --exact
cargo test -p compass-semantic-diff incomplete_test_evidence_never_claims_gap -- --exact
graphify update .
```

Expected: PASS; cycles terminate; limits fail closed; incomplete evidence never emits a test-gap claim.

- [ ] **Step 5: Commit impact and verification**

```bash
git add crates/compass-semantic-diff
git commit -m "feat: trace semantic impact and verification evidence"
```

---

### Task 12: Rank, Collapse, and Render the Actionable Report

**Files:**
- Create: `crates/compass-semantic-diff/src/rank.rs`
- Modify: `crates/compass-semantic-diff/src/lib.rs`
- Modify: `crates/compass-semantic-diff/tests/semantic_diff.rs`
- Create: `crates/compass-cli/src/semantic_diff_render.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Create: `crates/compass-cli/tests/semantic_diff_render.rs`
- Create: `crates/compass-cli/tests/golden/actionable.txt`
- Create: `crates/compass-cli/tests/golden/actionable.json`

**Interfaces:**
- Produces semantic-crate `rank_findings` plus CLI-owned renderers:

```rust
pub fn rank_findings(
    findings: Vec<SemanticFinding>,
) -> Result<(Vec<SemanticFinding>, Vec<CollapsedGroup>), SemanticDiffError>;

pub struct RenderOptions { pub include_routine: bool, pub explain: Option<String> }
pub fn render_text(report: &SemanticDiffReport, options: &RenderOptions) -> Result<String, SemanticDiffError>;
pub fn render_json(report: &SemanticDiffReport, options: &RenderOptions) -> Result<String, SemanticDiffError>;
```

- [ ] **Step 1: Add golden rendering tests**

Construct mixed findings and assert exact golden output. Default text must contain sections in this order:

```text
Semantic review: OLD -> NEW
N likely breaks · N behavior changes · N affected consumers · N test gaps
Likely breaks
Behavior changes
Affected callers and modules
Verification gaps
Routine changes collapsed: N (use --all)
Limitations
```

Assert JSON equals the complete golden schema object regardless of text display budget, default text collapses routine findings, `--all` expands them in text, `--explain sd1-0123456789abcdef01234567` renders one full finding, and unknown IDs return an error listing no unrelated details.

- [ ] **Step 2: Verify rendering modules are absent**

Run: `cargo test -p compass-cli --test semantic_diff_render renders_actionable_golden_report -- --exact`

Expected: compilation FAILS because rendering APIs do not exist.

- [ ] **Step 3: Implement ranking, collapse, and rendering**

In `compass-semantic-diff`, compute `review_priority` from compatibility, exact affected consumers, verification state, and completeness without averaging uncertainty. Sort by priority descending, confidence (`exact`, `probable`, `inferred`, `unknown`), affected-consumer count descending, subject, then ID. Put compatible internal additions/removals with no callers and location-only moves into `CollapsedGroup` entries. Retain digest-only implementation changes as indeterminate findings. Return `LimitExceeded { resource: "findings", limit: 5_000 }` before ranking finding 5,001.

In `compass-cli`, enforce a default text display budget of 20 actionable findings without hiding any `proven_break`; `--all` expands every retained collapsed finding. Serialize JSON through the complete report structs without applying the text budget:

```rust
serde_json::to_string_pretty(report)
    .map(|json| format!("{json}\n"))
    .map_err(SemanticDiffError::Serialize)
```

The explain renderer includes title, classification, explanation, affected entities, verification, evidence, and limitations for exactly one ID.

- [ ] **Step 4: Verify stable output**

Run:

```bash
cargo test -p compass-semantic-diff
cargo test -p compass-cli --test semantic_diff_render
for i in 1 2 3; do cargo test -p compass-cli --test semantic_diff_render renders_actionable_golden_report -- --exact; done
graphify update .
```

Expected: every run PASS; golden bytes remain unchanged.

- [ ] **Step 5: Commit report presentation**

```bash
git add crates/compass-semantic-diff crates/compass-cli/src/semantic_diff_render.rs crates/compass-cli/src/lib.rs crates/compass-cli/tests/semantic_diff_render.rs crates/compass-cli/tests/golden
git commit -m "feat: render actionable semantic diff reports"
```

---

### Task 13: Hard-Cut the `compass diff` CLI

**Files:**
- Modify: `crates/compass-cli/Cargo.toml`
- Create: `crates/compass-cli/src/semantic_diff_commands.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/bin/compass.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Consumes: `compare`, `render_text`, `render_json`, and existing history realization resolution.
- Produces: `pub(crate) fn command(frontend: Frontend, args: &[String]) -> Outcome`.

- [ ] **Step 1: Replace CLI integration assertions**

Add tests for default text, `--format json`, `--all`, `--explain`, `--fingerprint`, missing history, incomparable profiles, invalid finding ID, and rejection of every removed flag:

```rust
for removed in [
    "--detailed",
    "--topology-only",
    "--include-locations",
    "--include-analysis",
    "--include-metadata",
    "--allow-profile-mismatch",
    "--summarize",
] {
    let outcome = run_compass(["diff", "HEAD~1", "HEAD", removed]);
    assert_eq!(outcome.code, 2);
    assert!(outcome.stderr.contains("unknown option"));
}
```

Assert JSON `schema == "compass.semantic_diff.report/1"` and the output has no `changes`, `record_kind`, or legacy schema-version field.

- [ ] **Step 2: Verify tests fail against the legacy command**

Run: `cargo test -p compass-cli --test history_cli semantic_diff_ -- --nocapture`

Expected: FAIL because legacy flags/schema/rendering are still active.

- [ ] **Step 3: Add the replacement command and delete legacy code**

Move comparable-realization resolution into:

```rust
pub(crate) struct ResolvedComparison {
    pub repository: Repository,
    pub store: HistoryStore,
    pub old: PublishedVersion,
    pub new: PublishedVersion,
}

pub(crate) fn resolve_comparison(
    revisions: &[String],
    fingerprint: Option<&str>,
) -> Result<ResolvedComparison, String>;
```

Keep the existing exact-tree lazy materialization behavior inside `resolve_comparison`: resolve both commits without fetching, reuse valid preferred realizations, materialize a missing side with the selected normalized profile, and reject corrupt or cross-profile pairs before semantic adapters run.

Add a CLI-owned adapter:

```rust
struct HistorySnapshots<'a> {
    store: &'a HistoryStore,
    old: &'a RealizationId,
    new: &'a RealizationId,
}

impl SnapshotReader for HistorySnapshots<'_> {
    fn node(&self, side: SnapshotSide, id: &str) -> Result<Option<NodeRecord>, SemanticDiffError>;
    fn module(&self, side: SnapshotSide, path: &str) -> Result<Option<ModuleIr>, SemanticDiffError>;
    fn summary(&self, side: SnapshotSide, symbol: &str) -> Result<Option<FunctionSummary>, SemanticDiffError>;
    fn reverse_callers(&self, side: SnapshotSide, symbol: &str) -> Result<Vec<String>, SemanticDiffError>;
}
```

The command calls `Repository::source_delta`, streams `HistoryStore::diff` into a bounded collector that retains only changed node IDs and dependency edges, constructs `SemanticDiffInput`, then invokes `compare`. Convert history corruption and every `LimitExceeded` to command failure with no stdout.

Implement a strict parser accepting two revisions and only the MVP flags. Route `lib.rs` `"diff"` directly to `semantic_diff_commands::command`; make `src/bin/compass.rs` pass `diff` through the normal `run`/`write_outcome` path. Delete `DiffOutput`, `DiffOptions`, `parse_diff`, `render_diff`, `TextSink`, `JsonSink`, `ChangeCategory`, `command_diff_to_writer`, the public `run_diff` streaming special case, and their unit tests. Preserve `HistoryStore::diff` and `GraphChange` in `compass-history`.

Replace help with:

```text
Usage: compass diff <OLD> <NEW> [OPTIONS]

Options:
  --format <text|json>       Output format [default: text]
  --all                      Include routine symbol churn
  --explain <FINDING_ID>     Expand one semantic finding
  --fingerprint <SHA256>     Select one extraction fingerprint
```

- [ ] **Step 4: Verify the hard cutover**

Run:

```bash
cargo test -p compass-cli --test history_cli
cargo test -p compass-cli
cargo check --workspace
graphify update .
```

Expected: all tests PASS; removed flags exit `2`; `rg 'allow-profile-mismatch|topology-only|include-locations|DiffOutput|JsonSink' crates/compass-cli/src` returns no matches.

- [ ] **Step 5: Commit the CLI replacement**

```bash
git add crates/compass-cli Cargo.lock
git commit -m "feat: replace raw history diff with semantic review"
```

---

### Task 14: Update User Documentation for the Hard Cutover

**Files:**
- Modify: `README.md`
- Modify: `docs/reference/commands.md`
- Modify: `docs/guides/versioned-history.md`
- Modify: `docs/cookbook/impact-analysis.md`
- Modify: `docs/cookbook/ci-and-automation.md`
- Modify: `docs/roadmap.md`
- Modify: `crates/compass-cli/assets/compass-skill/references/history.md`

**Interfaces:**
- Consumes: final CLI and report vocabulary from Tasks 12–13.
- Produces: one consistent user workflow for building versioned graphs and reviewing semantic changes.

- [ ] **Step 1: Add a documentation contract check**

Run this before editing:

```bash
rg -n -- '--detailed|--topology-only|--include-locations|--include-analysis|--include-metadata|--allow-profile-mismatch|record_kind|schema_version.?2' README.md docs crates/compass-cli/assets
```

Expected: matches identify every legacy example to remove.

- [ ] **Step 2: Replace all command examples and explain evidence limits**

Use this canonical workflow everywhere:

```bash
compass history enable
compass history build main
compass history build HEAD --profile-from main
compass diff main HEAD
compass diff main HEAD --format json
compass diff main HEAD --all
compass diff main HEAD --explain sd1-0123456789abcdef01234567
```

Document the five actionable concepts, schema name, deterministic IDs, rebuild requirement for Program IR v2, and the rule that `partial`/`unknown` evidence cannot claim safety or a test gap. State that AI summaries and hosted PR delivery are outside this MVP rather than listing unsupported flags.

- [ ] **Step 3: Verify documentation and CLI help agree**

Run:

```bash
rg -n -- '--detailed|--topology-only|--include-locations|--include-analysis|--include-metadata|--allow-profile-mismatch|record_kind|schema_version.?2' README.md docs crates/compass-cli/assets
cargo run -q -p compass-cli --bin compass -- help diff
```

Expected: `rg` exits `1` with no matches; help shows only `--format`, `--all`, `--explain`, and `--fingerprint`.

- [ ] **Step 4: Run documentation-related tests**

Run:

```bash
cargo test -p compass-cli help
git diff --check
```

Expected: tests PASS; `git diff --check` emits no output.

- [ ] **Step 5: Commit the documentation cutover**

```bash
git add README.md docs crates/compass-cli/assets/compass-skill/references/history.md
git commit -m "docs: document actionable semantic history review"
```

---

### Task 15: Qualify the End-to-End MVP on Real Versioned Graphs

**Files:**
- Modify: `crates/compass-cli/tests/history_cli.rs`
- Create: `crates/compass-semantic-diff/tests/fixtures/python_contract/old.py`
- Create: `crates/compass-semantic-diff/tests/fixtures/python_contract/new.py`
- Create: `crates/compass-semantic-diff/tests/fixtures/rust_contract/old.rs`
- Create: `crates/compass-semantic-diff/tests/fixtures/rust_contract/new.rs`
- Create: `crates/compass-semantic-diff/tests/fixtures/typescript_contract/old.ts`
- Create: `crates/compass-semantic-diff/tests/fixtures/typescript_contract/new.ts`

**Interfaces:**
- Consumes: the complete Stage 0–1 implementation.
- Produces: repeatable acceptance coverage and recorded manual qualification commands; no benchmark fixture is committed from `cocoindex`.

- [ ] **Step 1: Add end-to-end language fixtures**

For each language, make `new` add a required parameter, change sync to async, add one dependency/call, add an error/effect, and leave one changed symbol as routine churn. The integration test must build both history realizations through the public build path and run the public CLI. A second case builds identical source/commit content in two different absolute checkout directories and asserts an empty finding list.

- [ ] **Step 2: Verify the end-to-end assertions**

Run: `cargo test -p compass-cli --test history_cli semantic_diff_end_to_end_languages -- --exact --nocapture`

Expected: PASS with findings for all five actionable categories; routine churn absent by default and present with `--all`.

- [ ] **Step 3: Run full automated verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
graphify update .
git diff --check
```

Expected: every command exits `0`; no warnings; graph report regenerates successfully.

- [ ] **Step 4: Rebuild and inspect the real cocoindex comparison**

First replay the established LevelDB cases from `/Volumes/Workspace/Github/leveldb`:

```bash
cd /Volumes/Workspace/Github/leveldb
/Users/haipingfu/graphify/compass/target/debug/compass history build 78a352f
/Users/haipingfu/graphify/compass/target/debug/compass history build 4a0c572 --profile-from 78a352f
/Users/haipingfu/graphify/compass/target/debug/compass diff 78a352f 4a0c572
/Users/haipingfu/graphify/compass/target/debug/compass history build bfae97f
/Users/haipingfu/graphify/compass/target/debug/compass history build 1d6e8d6 --profile-from bfae97f
/Users/haipingfu/graphify/compass/target/debug/compass diff bfae97f 1d6e8d6
git status --short
```

Expected: the deadlock body edit is either explained from supported summaries or explicitly indeterminate, never reduced to location churn; the Zstd addition reports retained symbols/call/dependency evidence; the original checkout remains clean.

From `/Volumes/Workspace/Github/cocoindex`, using the newly built Compass binary:

```bash
/Users/haipingfu/graphify/compass/target/debug/compass history build 90571539 --profile-from 71f9cc9
/Users/haipingfu/graphify/compass/target/debug/compass history build 71f9cc9 --profile-from 90571539
/Users/haipingfu/graphify/compass/target/debug/compass diff 90571539 71f9cc9
/Users/haipingfu/graphify/compass/target/debug/compass diff 90571539 71f9cc9 --format json > /tmp/compass-semantic-diff.json
jq -e '.schema == "compass.semantic_diff.report/1" and (.findings | type == "array")' /tmp/compass-semantic-diff.json
```

Expected: both builds succeed with one comparable fingerprint; text is concise rather than the prior 65 MB raw stream; JSON validation returns `true`; no `git_compass_tmp_worktree_` string appears:

```bash
! rg 'git_compass_tmp_worktree_' /tmp/compass-semantic-diff.json
```

- [ ] **Step 5: Commit qualification coverage**

```bash
git add crates/compass-cli/tests/history_cli.rs crates/compass-semantic-diff/tests/fixtures
git commit -m "test: qualify actionable semantic diff end to end"
```

---

## Acceptance Checklist

- `compass diff OLD NEW` reports actionable semantic findings, not raw storage records.
- Default ordering is likely breaks, behavior changes, affected callers/modules, verification gaps, then limitations.
- `--all`, `--format json`, `--explain`, and `--fingerprint` work exactly as documented.
- Every removed legacy option is rejected as unknown; no compatibility path remains.
- Report schema is exactly `compass.semantic_diff.report/1`.
- Stable finding inputs produce byte-identical IDs and JSON across physical checkouts.
- Python, Rust, TypeScript, TSX, and JavaScript callable contracts are represented in Program IR v2.
- Required-parameter, execution-mode, visibility, return-type, and parameter-kind changes use evidence-gated language rules.
- Effects, errors, calls, and dependency changes are correlated; digest-only changes stay indeterminate and location-only churn stays collapsed.
- Reverse-call traversal is breadth-first, cycle-safe, depth-limited, and edge-limited.
- A test gap is emitted only with complete test evidence.
- Real cocoindex output contains no temporary-worktree identity churn and is materially smaller than the prior raw diff.
- Workspace formatting, clippy, tests, graph refresh, and whitespace checks all pass.

## Explicit Follow-On Scope

Stage 2 will deepen cross-language dependency and impact precision, improve rename/move alignment, and add richer framework-specific test mapping. Stage 3 will separately design and implement optional AI summarization (`--summarize`) and hosted PR delivery. Neither follow-on may change the deterministic schema-v1 findings without a new report schema.
