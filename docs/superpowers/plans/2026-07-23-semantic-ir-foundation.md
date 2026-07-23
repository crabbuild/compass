# Semantic IR Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make native Compass builds emit, cache, validate, summarize, and historically preserve a deterministic semantic-program artifact for Rust and TypeScript/JavaScript source files.

**Architecture:** Add a dependency-light `compass-ir` crate for the versioned language-neutral schema and a `compass-analysis` crate for deterministic per-function summaries and reverse dependencies. `compass-languages` supplies seed Rust and TypeScript adapters, `compass-core` orchestrates path-safe content-addressed per-file analysis and atomically writes `program.json`, and `compass-history` stores program records in a sixth Prolly tree while retaining read compatibility with schema-2 realizations.

**Tech Stack:** Rust 1.97.1, Rust 2024 edition, Serde/JSON, SHA-256, tree-sitter 0.26, Rayon, `prolly-map = "=0.5.0"`, `prolly-store-sqlite = "=0.3.0"`, the existing Compass cache, graph pipeline, and history store.

## Global Constraints

- Implement inside the standalone Compass repository at `/Users/haipingfu/graphify/compass`.
- Preserve the workspace's `unsafe_code = "forbid"` and denied Clippy lint policy.
- Structural extraction for every existing language must remain unchanged.
- Rust and TypeScript/JavaScript are the only deep-tier seed adapters in this plan.
- Static facts, derived summaries, runtime observations, hypotheses, and agent assertions remain separate record classes; this plan creates only static facts and deterministic derived summaries.
- Every derived summary carries its source symbol, schema versions, completeness, and deterministic digests.
- Unsupported source files produce no program module; supported but incompletely modeled constructs produce explicit partial reasons.
- Repository-relative source identity, source bytes, adapter version, and analyzer version are meaning-affecting inputs; absolute checkout paths never enter program artifacts.
- Two files with identical bytes but different repository-relative paths must have distinct cache entries, module identities, and symbol identities.
- Identical repository content and analyzer versions in different checkout roots must produce byte-equivalent canonical program artifacts.
- Incremental output must equal a clean full build.
- Program artifacts use their own versioned Prolly root and share unchanged content across realizations.
- Program IR schema, adapter algorithm, summary schema, and analyzer algorithm versions participate in cache namespaces and history extraction fingerprints.
- Graphify compatibility mode must not emit `program.json` or change its legacy output file set.
- Do not add network access, a model call, runtime telemetry collection, a graph database server, or autonomous editing.
- After code changes, run `compass update .` from the Compass repository to refresh `compass-out/`, then run `graphify update .` from `/Users/haipingfu/graphify` to refresh the superproject graph required by its `AGENTS.md`.

---

## Scope boundary

This plan implements the first independently testable subproject from the
technical-moat roadmap. It does not implement interprocedural fixed-point data
flow, runtime evidence overlays, `compass impact`, federated repositories, or
agent APIs. Those capabilities consume the stable interfaces produced here and
receive separate specs and plans.

The delivered artifact is:

```text
compass-out/program.json
```

Its schema is `compass.program/1`. It contains normalized per-file IR,
deterministic function summaries, and a reverse-call index. It is authoritative
static analysis state and participates in history realization identity.

Because this capability has not shipped, the rename is a clean cut:
implementations must never read, write, register, reserve, document, or migrate
`.compass_program.json`. The only artifact name is `program.json`, under an
output directory owned by Compass (`compass-out/`) or Graphify compatibility
mode (`graphify-out/`, where program analysis remains disabled).

The artifact lifecycle is complete only when all of these paths agree on that
name and the same canonical value:

1. native pipeline production and unchanged-build validation;
2. per-file cache reconstruction;
3. history ingestion, partitioning, publication, reopen, and materialization;
4. full diff, topology-only exclusion, structural sharing, and garbage
   collection;
5. `history export --format compass-out`;
6. documentation and clean-versus-incremental qualification.

## File and responsibility map

### New crates

- `crates/compass-ir/Cargo.toml`: package metadata and minimal serialization dependencies.
- `crates/compass-ir/src/lib.rs`: public exports and schema constants.
- `crates/compass-ir/src/model.rs`: language-neutral IR and completeness types.
- `crates/compass-ir/src/canonical.rs`: canonical ordering, bytes, and digest.
- `crates/compass-ir/src/validation.rs`: structural invariants and typed errors.
- `crates/compass-ir/tests/schema.rs`: serialization, determinism, and rejection coverage.
- `crates/compass-analysis/Cargo.toml`: summary engine package metadata.
- `crates/compass-analysis/src/lib.rs`: public summary and invalidation API.
- `crates/compass-analysis/src/summary.rs`: per-function behavior summaries.
- `crates/compass-analysis/src/invalidation.rs`: reverse dependencies and affected-summary closure.
- `crates/compass-analysis/tests/summary.rs`: summary and incremental-equivalence coverage.

### Language adapters

- `crates/compass-languages/src/program/mod.rs`: supported-language dispatch and adapter contract.
- `crates/compass-languages/src/program/rust.rs`: Rust seed adapter.
- `crates/compass-languages/src/program/typescript.rs`: TypeScript, TSX, and JavaScript seed adapter.
- `crates/compass-languages/tests/program_ir.rs`: adapter fixtures and explicit partial-state tests.

### Pipeline and cache

- `crates/compass-files/src/cache.rs`: versioned `Program` cache kind.
- `crates/compass-files/tests/contracts.rs`: program-cache isolation and clearing.
- `crates/compass-core/src/program.rs`: program analysis collection, canonical merge, and sidecar writing.
- `crates/compass-core/src/pipeline.rs`: invoke program analysis beside AST extraction and include counts in `BuildResult`.
- `crates/compass-core/src/lib.rs`: export program build result types.
- `crates/compass-core/tests/program_pipeline.rs`: cold, warm, changed-file, deleted-file, and clean-build equivalence.
- `crates/compass-output/src/backup.rs`: preserve `program.json` with protected authoritative output.

### History

- `crates/compass-history/src/model.rs`: optional schema-3 program root and count with schema-2 compatibility.
- `crates/compass-history/src/artifacts.rs`: load, partition, reconstruct, register, and write the program artifact.
- `crates/compass-history/src/keys.rs`: typed program keys.
- `crates/compass-history/src/store.rs`: build, publish, open, and verify the program tree.
- `crates/compass-history/src/validate.rs`: scan and validate program records and counts.
- `crates/compass-history/src/diff.rs`: stream exact program-record changes in full diffs.
- `crates/compass-history/src/gc.rs`: retain and prune schema-aware program named roots.
- `crates/compass-history/tests/roundtrip.rs`: artifact round trip.
- `crates/compass-history/tests/publication.rs`: schema-3 publication and schema-2 reopen.
- `crates/compass-history/tests/fixtures/schema2_graph_version.json`: pre-schema-3 canonical manifest golden.
- `crates/compass-history/tests/performance.rs`: structural sharing for unchanged summaries.
- `crates/compass-history/tests/diff.rs`: program-record diff and topology-only exclusion.
- `crates/compass-history/tests/maintenance.rs`: mixed schema-2/schema-3 garbage collection.

### Product qualification

- `crates/compass-core/src/history.rs`: include program algorithm versions in exact-checkout fingerprints.
- `crates/compass-cli/src/lib.rs`: enable program analysis only for the Compass frontend and initialize every `BuildResult` fixture.
- `crates/compass-cli/src/history_build.rs`: persist and validate program algorithm versions in history build profiles.
- `crates/compass-cli/src/history_commands.rs`: include `program.json` in `history export --format compass-out`.
- `crates/compass-cli/tests/program_cli.rs`: native CLI artifact and compatibility isolation.
- `crates/compass-output/src/history_bundle.rs`: write and validate the authoritative `program.json` during history export.
- `crates/compass-output/tests/history_bundle.rs`: exported program artifact equivalence and reserved-path coverage.
- `scripts/qualify_program_foundation.sh`: clean/incremental equivalence qualification.
- `README.md`: program artifact and language-tier documentation.
- `PERFORMANCE.md`: qualification command and initial baseline fields.

## Public interfaces fixed by this plan

```rust
// compass-ir
pub const PROGRAM_SCHEMA_VERSION: u32 = 1;
pub type SymbolId = String;

pub struct ProgramBundle {
    pub schema_version: u32,
    pub adapter_version: u32,
    pub modules: Vec<ModuleIr>,
}

pub struct ModuleIr {
    pub language: String,
    pub source_file: String,
    pub source_digest: String,
    pub functions: Vec<FunctionIr>,
}

pub struct FunctionIr {
    pub symbol_id: SymbolId,
    pub graph_node_id: String,
    pub name: String,
    pub span: SourceSpan,
    pub signature_digest: String,
    pub implementation_digest: String,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<TypeRef>,
    pub blocks: Vec<BasicBlock>,
    pub completeness: Completeness,
}

// compass-analysis
pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;
pub const ANALYZER_VERSION: u32 = 1;

pub struct AnalysisBundle {
    pub schema_version: u32,
    pub ir_schema_version: u32,
    pub analyzer_version: u32,
    pub program: ProgramBundle,
    pub summaries: Vec<BehaviorSummary>,
    pub reverse_calls: BTreeMap<SymbolId, Vec<SymbolId>>,
}

pub fn analyze(program: ProgramBundle) -> Result<AnalysisBundle, AnalysisError>;
impl AnalysisBundle {
    pub fn validate(&self) -> Result<(), AnalysisError>;
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, AnalysisError>;
}
pub fn invalidation_plan(
    previous: &AnalysisBundle,
    current: &AnalysisBundle,
) -> InvalidationPlan;

// compass-languages
pub const PROGRAM_ADAPTER_VERSION: u32 = 1;

impl Engine {
    pub fn program_ir_source(
        &mut self,
        path: &Path,
        source: &[u8],
    ) -> Result<Option<ModuleIr>, ExtractError>;
}
```

### Task 1: Add the versioned semantic IR crate

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/compass-ir/Cargo.toml`
- Create: `crates/compass-ir/src/lib.rs`
- Create: `crates/compass-ir/src/model.rs`
- Create: `crates/compass-ir/src/canonical.rs`
- Create: `crates/compass-ir/src/validation.rs`
- Create: `crates/compass-ir/tests/schema.rs`

**Interfaces:**
- Consumes: Serde, `serde_json`, SHA-256, and `thiserror`.
- Produces: `ProgramBundle`, `ModuleIr`, `FunctionIr`, CFG operation types, `Completeness`, `IrError`, `canonical_bytes`, and `digest`.

- [ ] **Step 1: Write the schema contract tests**

Create `crates/compass-ir/tests/schema.rs` with fixtures that exercise ordering,
CFG targets, completeness, and digests:

```rust
use compass_ir::{
    BasicBlock, Completeness, FunctionIr, ModuleIr, Operation, OperationKind, ProgramBundle,
    SourceSpan, Terminator, PROGRAM_SCHEMA_VERSION,
};

fn function(symbol: &str, callee: &str) -> FunctionIr {
    FunctionIr {
        symbol_id: symbol.to_owned(),
        graph_node_id: symbol.to_owned(),
        name: symbol.to_owned(),
        span: SourceSpan::lines(1, 3),
        signature_digest: "a".repeat(64),
        implementation_digest: "b".repeat(64),
        parameters: Vec::new(),
        return_type: None,
        blocks: vec![BasicBlock {
            id: 0,
            operations: vec![Operation {
                ordinal: 0,
                span: SourceSpan::lines(2, 2),
                kind: OperationKind::Call {
                    callee: callee.to_owned(),
                    resolved_symbol: None,
                    receiver_type: None,
                },
            }],
            terminator: Terminator::Return,
        }],
        completeness: Completeness::Complete,
    }
}

fn bundle(functions: Vec<FunctionIr>) -> ProgramBundle {
    ProgramBundle {
        schema_version: PROGRAM_SCHEMA_VERSION,
        adapter_version: 1,
        modules: vec![ModuleIr {
            language: "rust".to_owned(),
            source_file: "src/lib.rs".to_owned(),
            source_digest: "c".repeat(64),
            functions,
        }],
    }
}

#[test]
fn canonical_bytes_ignore_module_and_function_insertion_order()
-> Result<(), Box<dyn std::error::Error>> {
    let first = bundle(vec![function("b", "a"), function("a", "external")]);
    let second = bundle(vec![function("a", "external"), function("b", "a")]);
    assert_eq!(first.canonical_bytes()?, second.canonical_bytes()?);
    assert_eq!(first.digest()?, second.digest()?);
    Ok(())
}

#[test]
fn validation_rejects_unknown_cfg_targets_and_duplicate_symbols() {
    let mut invalid_target = function("a", "external");
    invalid_target.blocks[0].terminator = Terminator::Goto { target: 7 };
    assert!(bundle(vec![invalid_target]).validate().is_err());
    assert!(bundle(vec![function("a", "x"), function("a", "y")])
        .validate()
        .is_err());
}

#[test]
fn partial_reasons_are_nonempty_and_canonical()
-> Result<(), Box<dyn std::error::Error>> {
    let mut item = function("a", "external");
    item.completeness = Completeness::Partial {
        reasons: vec!["reflection".to_owned(), "dynamic dispatch".to_owned()],
    };
    let value = bundle(vec![item]).canonicalized()?;
    assert_eq!(
        value.modules[0].functions[0].completeness,
        Completeness::Partial {
            reasons: vec!["dynamic dispatch".to_owned(), "reflection".to_owned()],
        }
    );
    Ok(())
}
```

Validation must reject `schema_version != PROGRAM_SCHEMA_VERSION`,
`adapter_version == 0`, absolute or non-normalized `source_file` values,
duplicate module paths, duplicate symbol IDs, invalid digests, empty partial
reasons, and invalid CFG targets. An empty `graph_node_id` is permitted only
when the function is partial with reason `missing_graph_identity`.

- [ ] **Step 2: Run the tests and verify the crate is absent**

Run:

```bash
cargo test -p compass-ir --test schema
```

Expected: Cargo fails because package `compass-ir` does not exist.

- [ ] **Step 3: Add the workspace package and schema types**

Add `"crates/compass-ir"` to the root workspace members. Create
`crates/compass-ir/Cargo.toml`:

```toml
[package]
name = "compass-ir"
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
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true

[lints]
workspace = true
```

Define the model in `src/model.rs`. Use `snake_case` enum serialization and
ordered collections only where the order has semantic meaning:

```rust
use serde::{Deserialize, Serialize};

pub type SymbolId = String;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramBundle {
    pub schema_version: u32,
    pub adapter_version: u32,
    pub modules: Vec<ModuleIr>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModuleIr {
    pub language: String,
    pub source_file: String,
    pub source_digest: String,
    pub functions: Vec<FunctionIr>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FunctionIr {
    pub symbol_id: SymbolId,
    pub graph_node_id: String,
    pub name: String,
    pub span: SourceSpan,
    pub signature_digest: String,
    pub implementation_digest: String,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<TypeRef>,
    pub blocks: Vec<BasicBlock>,
    pub completeness: Completeness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub type_ref: Option<TypeRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeRef {
    pub display: String,
    pub resolved_symbol: Option<SymbolId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

impl SourceSpan {
    #[must_use]
    pub const fn lines(start_line: u32, end_line: u32) -> Self {
        Self {
            start_line,
            start_column: 0,
            end_line,
            end_column: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BasicBlock {
    pub id: u32,
    pub operations: Vec<Operation>,
    pub terminator: Terminator,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub ordinal: u32,
    pub span: SourceSpan,
    pub kind: OperationKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationKind {
    Call {
        callee: String,
        resolved_symbol: Option<SymbolId>,
        receiver_type: Option<String>,
    },
    Read { path: String },
    Write { path: String },
    Await,
    Throw,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Terminator {
    Return,
    Goto { target: u32 },
    Branch { then_target: u32, else_target: u32 },
    Unreachable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Completeness {
    Complete,
    Partial { reasons: Vec<String> },
}
```

- [ ] **Step 4: Implement validation and canonicalization**

In `src/validation.rs`, validate schema version `1`, lowercase SHA-256 text,
unique source files and symbols, nonempty partial reasons, contiguous operation
ordinals, unique block IDs, block zero, and valid branch targets. Define exact
typed failures:

```rust
#[derive(Debug, thiserror::Error)]
pub enum IrError {
    #[error("unsupported program schema {0}")]
    Schema(u32),
    #[error("duplicate source file {0}")]
    DuplicateSource(String),
    #[error("duplicate symbol {0}")]
    DuplicateSymbol(String),
    #[error("function {symbol} has no entry block 0")]
    MissingEntry { symbol: String },
    #[error("function {symbol} block {block} targets missing block {target}")]
    MissingTarget {
        symbol: String,
        block: u32,
        target: u32,
    },
    #[error("function {symbol} block {block} has non-contiguous operation ordinals")]
    OperationOrder { symbol: String, block: u32 },
    #[error("partial analysis for {0} has no reason")]
    EmptyPartialReason(String),
    #[error("invalid SHA-256 digest in {field} for {owner}")]
    Digest {
        owner: String,
        field: &'static str,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
```

In `src/canonical.rs`, clone and sort modules by `source_file`, functions by
`symbol_id`, blocks by `id`, partial reasons lexicographically, and JSON object
keys recursively. Do not reorder parameters, operations, or control-flow
targets. Hash the canonical bytes with SHA-256.

In `src/lib.rs`, re-export the types and implement:

```rust
pub const PROGRAM_SCHEMA_VERSION: u32 = 1;

impl ProgramBundle {
    pub fn validate(&self) -> Result<(), IrError>;
    pub fn canonicalized(&self) -> Result<Self, IrError>;
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, IrError>;
    pub fn digest(&self) -> Result<String, IrError>;
}
```

- [ ] **Step 5: Run schema tests, formatting, and Clippy**

Run:

```bash
cargo test -p compass-ir
cargo fmt --check
cargo clippy -p compass-ir --all-targets -- -D warnings
```

Expected: all commands succeed.

- [ ] **Step 6: Commit the IR contract**

```bash
git add Cargo.toml Cargo.lock crates/compass-ir
git commit -m "feat(ir): add versioned semantic program schema"
```

### Task 2: Add deterministic behavior summaries and invalidation

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/compass-analysis/Cargo.toml`
- Create: `crates/compass-analysis/src/lib.rs`
- Create: `crates/compass-analysis/src/summary.rs`
- Create: `crates/compass-analysis/src/invalidation.rs`
- Create: `crates/compass-analysis/tests/summary.rs`

**Interfaces:**
- Consumes: `compass_ir::ProgramBundle`.
- Produces: `AnalysisBundle`, canonical program-artifact bytes, `BehaviorSummary`, `EffectKind`, `AnalysisError`, `analyze`, `InvalidationPlan`, and `invalidation_plan`.

- [ ] **Step 1: Write failing summary and invalidation tests**

Create `crates/compass-analysis/tests/summary.rs`:

```rust
use compass_analysis::{EffectKind, analyze, invalidation_plan};
use compass_ir::{
    BasicBlock, Completeness, FunctionIr, ModuleIr, Operation, OperationKind, ProgramBundle,
    SourceSpan, Terminator, PROGRAM_SCHEMA_VERSION,
};

fn program(body_digest: char, call: &str) -> ProgramBundle {
    ProgramBundle {
        schema_version: PROGRAM_SCHEMA_VERSION,
        adapter_version: 1,
        modules: vec![ModuleIr {
            language: "rust".to_owned(),
            source_file: "src/lib.rs".to_owned(),
            source_digest: "c".repeat(64),
            functions: vec![
                FunctionIr {
                    symbol_id: "caller".to_owned(),
                    graph_node_id: "caller".to_owned(),
                    name: "caller".to_owned(),
                    span: SourceSpan::lines(1, 4),
                    signature_digest: "a".repeat(64),
                    implementation_digest: body_digest.to_string().repeat(64),
                    parameters: Vec::new(),
                    return_type: None,
                    blocks: vec![BasicBlock {
                        id: 0,
                        operations: vec![
                            Operation {
                                ordinal: 0,
                                span: SourceSpan::lines(2, 2),
                                kind: OperationKind::Call {
                                    callee: call.to_owned(),
                                    resolved_symbol: Some(call.to_owned()),
                                    receiver_type: None,
                                },
                            },
                            Operation {
                                ordinal: 1,
                                span: SourceSpan::lines(3, 3),
                                kind: OperationKind::Await,
                            },
                        ],
                        terminator: Terminator::Return,
                    }],
                    completeness: Completeness::Complete,
                },
            ],
        }],
    }
}

#[test]
fn summaries_collect_calls_effects_and_reverse_edges()
-> Result<(), Box<dyn std::error::Error>> {
    let analysis = analyze(program('b', "callee"))?;
    let summary = &analysis.summaries[0];
    assert_eq!(
        summary.resolved_callees,
        std::collections::BTreeSet::from(["callee".to_owned()])
    );
    assert!(summary.effects.contains(&EffectKind::Awaits));
    assert_eq!(analysis.reverse_calls["callee"], ["caller"]);
    Ok(())
}

#[test]
fn invalidation_closes_over_reverse_callers()
-> Result<(), Box<dyn std::error::Error>> {
    let previous = analyze(program('b', "callee"))?;
    let current = analyze(program('d', "callee"))?;
    let plan = invalidation_plan(&previous, &current);
    assert_eq!(
        plan.changed,
        std::collections::BTreeSet::from(["caller".to_owned()])
    );
    assert_eq!(
        plan.affected,
        std::collections::BTreeSet::from(["caller".to_owned()])
    );
    Ok(())
}
```

- [ ] **Step 2: Run the tests and verify the package is absent**

Run:

```bash
cargo test -p compass-analysis --test summary
```

Expected: Cargo fails because package `compass-analysis` does not exist.

- [ ] **Step 3: Add summary model and aggregation**

Create the package with dependencies on `compass-ir`, Serde, SHA-256,
`serde_json`, and `thiserror`. Define:

```rust
use std::collections::{BTreeMap, BTreeSet};
use compass_ir::{Completeness, ProgramBundle, SymbolId};
use serde::{Deserialize, Serialize};

pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;
pub const ANALYZER_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    ReadsState,
    WritesState,
    Awaits,
    MayThrow,
    CallsUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BehaviorSummary {
    pub symbol_id: SymbolId,
    pub signature_digest: String,
    pub implementation_digest: String,
    pub resolved_callees: BTreeSet<SymbolId>,
    pub unresolved_callees: BTreeSet<String>,
    pub reads: BTreeSet<String>,
    pub writes: BTreeSet<String>,
    pub effects: BTreeSet<EffectKind>,
    pub completeness: Completeness,
    pub summary_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisBundle {
    pub schema_version: u32,
    pub ir_schema_version: u32,
    pub analyzer_version: u32,
    pub program: ProgramBundle,
    pub summaries: Vec<BehaviorSummary>,
    pub reverse_calls: BTreeMap<SymbolId, Vec<SymbolId>>,
}

#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    #[error(transparent)]
    Ir(#[from] compass_ir::IrError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid analysis bundle: {0}")]
    InvalidBundle(String),
}
```

`ANALYSIS_SCHEMA_VERSION` versions the serialized result shape.
`ANALYZER_VERSION` versions meaning-affecting summary behavior even when the
shape does not change and is serialized into `AnalysisBundle.analyzer_version`.
`ProgramBundle.adapter_version` records the meaning-affecting producer version
supplied by the pipeline. `analyze` validates and canonicalizes the program,
visits operations in block and ordinal order, collects call/read/write/effect sets, computes a digest from
the summary with an empty `summary_digest`, sorts summaries by `symbol_id`, and
deduplicates every reverse-call caller list.

`AnalysisBundle::validate` must prove:

- current analysis, IR, and analyzer versions plus a valid nested program;
- exactly one summary for every function and no summary for an unknown symbol;
- summary signature, implementation, and completeness fields match their
  source function;
- every `summary_digest` recomputes exactly;
- every resolved callee names a program symbol; and
- `reverse_calls` is the exact duplicate-free inverse of resolved calls,
  independent of input vector order.

Add negative tests for a forged summary digest, a missing summary, a dangling
resolved callee, and an inconsistent reverse-call entry.

`AnalysisBundle::canonical_bytes` validates current IR/analysis schemas, a
nonzero recorded adapter version, and the current analyzer version;
canonicalizes a clone of its nested program, summaries, sets, and reverse-call
vectors; validates that canonical clone; then returns compact UTF-8
`serde_json` bytes with no trailing newline.
It is the sole byte contract for native output, history registry digests, and
history export. Add a test with non-ASCII identifiers and reversed insertion
order that asserts two equivalent bundles produce identical bytes.

- [ ] **Step 4: Implement deterministic invalidation**

Define:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InvalidationPlan {
    pub added: BTreeSet<SymbolId>,
    pub removed: BTreeSet<SymbolId>,
    pub changed: BTreeSet<SymbolId>,
    pub affected: BTreeSet<SymbolId>,
}
```

`invalidation_plan` compares `summary_digest` by symbol. Seed `affected` with
added, removed, and changed symbols, then breadth-first traverse the union of
the previous and current `reverse_calls` maps until no new caller is added.
Use a `VecDeque` and `BTreeSet` so output is deterministic.

- [ ] **Step 5: Run focused and workspace-compatible checks**

Run:

```bash
cargo test -p compass-analysis
cargo fmt --check
cargo clippy -p compass-analysis --all-targets -- -D warnings
```

Expected: all commands succeed.

- [ ] **Step 6: Commit the analysis engine**

```bash
git add Cargo.toml Cargo.lock crates/compass-analysis
git commit -m "feat(analysis): summarize program behavior and invalidation"
```

### Task 3: Add a schema-isolated program cache

**Files:**
- Modify: `crates/compass-files/src/cache.rs`
- Modify: `crates/compass-files/tests/contracts.rs`

**Interfaces:**
- Consumes: existing content hashing, atomic JSON writes, and path normalization.
- Produces: `CacheKind::Program { ir_schema: u32, adapter_version: u32, analysis_schema: u32, analyzer_version: u32 }` with repository-relative, path-sensitive entry identities and live-key pruning.

- [ ] **Step 1: Add a failing cache-isolation test**

Append to `crates/compass-files/tests/contracts.rs`:

```rust
#[test]
fn program_cache_is_path_sensitive_and_version_isolated()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("a/main.rs");
    let second = directory.path().join("b/main.rs");
    std::fs::create_dir_all(first.parent().ok_or("first parent missing")?)?;
    std::fs::create_dir_all(second.parent().ok_or("second parent missing")?)?;
    std::fs::write(&first, "fn main() {}\n")?;
    std::fs::write(&second, "fn main() {}\n")?;
    let mut cache = Cache::new(directory.path(), None)?;
    let v1 = CacheKind::Program {
        ir_schema: 1,
        adapter_version: 1,
        analysis_schema: 1,
        analyzer_version: 1,
    };
    let v2 = CacheKind::Program {
        ir_schema: 1,
        adapter_version: 2,
        analysis_schema: 1,
        analyzer_version: 1,
    };
    cache.save(
        &first,
        &serde_json::json!({"source_file": "a/main.rs"}),
        &v1,
        None,
    )?;
    cache.save(
        &second,
        &serde_json::json!({"source_file": "b/main.rs"}),
        &v1,
        None,
    )?;
    let first_value = cache
        .load(&first, &v1, None, false, false)?
        .ok_or("first program cache entry missing")?;
    let second_value = cache
        .load(&second, &v1, None, false, false)?
        .ok_or("second program cache entry missing")?;
    assert_eq!(
        first_value["source_file"].as_str(),
        Some("a/main.rs")
    );
    assert_eq!(
        second_value["source_file"].as_str(),
        Some("b/main.rs")
    );
    assert!(cache.load(&first, &v2, None, false, false)?.is_none());
    assert_ne!(cache.directory(&v1, None), cache.directory(&v2, None));
    cache.clear();
    assert!(cache.load(&first, &v1, None, false, false)?.is_none());
    Ok(())
}
```

Add a second case with two different checkout roots, one shared external cache
root, and the same `src/lib.rs` bytes. Save from the first checkout, load from
the second, and assert the cache hits while the stored and loaded
`source_file` remains exactly `src/lib.rs`.

Add a pruning case that saves a program entry, changes that file, saves its new
entry, deletes a second cached source, and calls:

```rust
cache.prune_program(&v1, &[changed.clone()])?
```

Assert the changed file's current entry remains, its old-content entry and the
deleted-file entry are removed, and other cache kinds are untouched.

- [ ] **Step 2: Run the test and verify the enum variant is absent**

Run:

```bash
cargo test -p compass-files --test contracts program_cache_is_path_sensitive
```

Expected: compilation fails because `CacheKind::Program` is undefined.

- [ ] **Step 3: Implement the program cache namespace**

Extend `CacheKind`:

```rust
Program {
    ir_schema: u32,
    adapter_version: u32,
    analysis_schema: u32,
    analyzer_version: u32,
},
```

Map it to:

```rust
format!(
    "program/ir{ir_schema}-adapter{adapter_version}-analysis{analysis_schema}-analyzer{analyzer_version}"
)
```

Add a private `entry_key(&mut self, path: &Path, kind: &CacheKind)` used by
`load`, `save`, and `save_batch`. Existing kinds retain the current content hash.
For `Program`, compute lowercase SHA-256 over:

```text
normalized repository-relative path bytes
0x00
content hash bytes
```

Normalize separators to `/` and reject a path outside `Cache::root`. Update
`cached_files` and `clear` to recurse through `cache/program`. Do not allow
semantic prompt fingerprints or AST extractor-version fallback for this kind.
The existing generic cache transforms absolute `source_file` values on save
and load; bypass both transforms for `Program`. Program values must already
contain the normalized repository-relative path used by `entry_key`, and
`save`/`save_batch` reject a mismatched, absolute, or non-normalized
`source_file`.
On `load`, treat a missing or mismatched program `source_file` as a cache miss
rather than returning it.

Implement:

```rust
pub fn prune_program(
    &mut self,
    kind: &CacheKind,
    live_paths: &[PathBuf],
) -> Result<usize, FileError>;
```

Require `kind` to be `Program`, compute the current live `entry_key` set, and
delete only obsolete JSON entries in that exact version directory. Missing live
files are excluded. Never prune `Ast` or semantic namespaces through this API;
`clear` remains the explicit all-version cleanup.
Keep the existing `Ast`, `Semantic`, and `SemanticMode` paths byte-for-byte
unchanged.

- [ ] **Step 4: Run cache and file-contract tests**

Run:

```bash
cargo test -p compass-files --test contracts
cargo clippy -p compass-files --all-targets -- -D warnings
```

Expected: all tests pass.

- [ ] **Step 5: Commit the cache contract**

```bash
git add crates/compass-files/src/cache.rs crates/compass-files/tests/contracts.rs
git commit -m "feat(files): add versioned program analysis cache"
```

### Task 4: Add the adapter contract and Rust seed IR

**Files:**
- Modify: `crates/compass-languages/Cargo.toml`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/engine.rs`
- Create: `crates/compass-languages/src/program/mod.rs`
- Create: `crates/compass-languages/src/program/rust.rs`
- Create: `crates/compass-languages/tests/program_ir.rs`

**Interfaces:**
- Consumes: `compass-ir`, the existing registry, parser cache, tree-sitter trees, `make_id`, and source bytes.
- Produces: `PROGRAM_ADAPTER_VERSION`, `Engine::program_ir_source`, and Rust `ModuleIr` records.

- [ ] **Step 1: Write failing Rust adapter tests**

Create `crates/compass-languages/tests/program_ir.rs`:

```rust
use std::path::Path;
use compass_ir::{Completeness, OperationKind};
use compass_languages::Engine;

#[test]
fn rust_program_ir_captures_functions_calls_state_and_await()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
async fn load(state: &mut State) -> Result<Item, Error> {
    state.count = state.count + 1;
    let item = fetch(state.id).await?;
    Ok(item)
}
"#;
    let module = Engine::default()
        .program_ir_source(Path::new("src/lib.rs"), source)?
        .ok_or("Rust adapter returned no module")?;
    assert_eq!(module.language, "rust");
    let function = &module.functions[0];
    assert_eq!(function.name, "load");
    assert_eq!(function.blocks[0].id, 0);
    let operations = function
        .blocks
        .iter()
        .flat_map(|block| &block.operations)
        .map(|operation| &operation.kind)
        .collect::<Vec<_>>();
    assert!(operations.iter().any(|kind| matches!(
        kind,
        OperationKind::Call { callee, .. } if callee == "fetch"
    )));
    assert!(operations.iter().any(|kind| matches!(kind, OperationKind::Write { .. })));
    assert!(operations.iter().any(|kind| matches!(kind, OperationKind::Await)));
    assert!(matches!(
        function.completeness,
        Completeness::Partial { ref reasons }
            if reasons.iter().any(|reason| reason == "question_mark_control_flow")
    ));
    Ok(())
}

#[test]
fn unsupported_languages_return_no_program_module()
-> Result<(), Box<dyn std::error::Error>> {
    let module = Engine::default()
        .program_ir_source(Path::new("main.go"), b"package main")
        ?;
    assert!(module.is_none());
    Ok(())
}
```

- [ ] **Step 2: Run the tests and verify the API is absent**

Run:

```bash
cargo test -p compass-languages --test program_ir rust_program_ir
```

Expected: compilation fails because `program_ir_source` is undefined.

- [ ] **Step 3: Add the program dispatch API**

Add `compass-ir` to `compass-languages/Cargo.toml`. In `program/mod.rs`, expose
crate-local adapters:

```rust
mod rust;
mod typescript;

use std::path::Path;
use compass_ir::ModuleIr;
use tree_sitter::Node;

pub const PROGRAM_ADAPTER_VERSION: u32 = 1;

pub(crate) fn extract(
    language: &str,
    path: &Path,
    source: &[u8],
    root: Node<'_>,
) -> Option<ModuleIr> {
    match language {
        "rust" => Some(rust::extract(path, source, root)),
        "javascript" | "typescript" | "tsx" => {
            Some(typescript::extract(language, path, source, root))
        }
        _ => None,
    }
}
```

In `Engine`, implement `program_ir_source` by resolving the existing
`LanguageSpec`, returning `Ok(None)` before parsing unsupported languages,
parsing with the existing cached parser, invoking `program::extract`, then
validating the one-module bundle. Map an invalid adapter result to:

```rust
ExtractError::InvalidProgram {
    path: path.to_path_buf(),
    detail: error.to_string(),
}
```

Add that exact variant to `ExtractError`.

Adapters reproduce the existing structural extractor's `make_id` inputs when a
function has a structural graph node. If no exact existing identity can be
proven, emit an empty `graph_node_id` and add `missing_graph_identity`; never
invent a dangling graph reference.

The public re-export in `compass-languages/src/lib.rs` is:

```rust
pub use program::PROGRAM_ADAPTER_VERSION;
```

- [ ] **Step 4: Implement the Rust seed adapter**

The Rust adapter must:

- create one `FunctionIr` for free functions and `impl` methods;
- use the same `make_id` inputs as `rust_lang.rs` for `graph_node_id`;
- receive a normalized repository-relative path with `/` separators from
  `compass-core`, never an absolute checkout path;
- derive `symbol_id` from that normalized source path, the fully qualified impl
  owner (`Type` or `<Type as Trait>`), function name, and signature digest;
- hash the signature range and body range separately with SHA-256;
- preserve parameter order and return-type spelling;
- emit a deterministic entry block `0`;
- emit calls, field reads, assignment writes, `await`, explicit `return`, and
  explicit panic/error macro calls;
- mark `?`, macro-expanded behavior, trait dispatch, and reflection-like calls
  as named partial reasons;
- preserve source order through operation ordinals.
- pre-collect same-module definitions and set `resolved_symbol` only when an
  unqualified call has exactly one same-module target; every other call remains
  unresolved.

The existing structural Rust extractor intentionally collapses some trait-impl
methods onto the same `graph_node_id`. Do not change that extractor in this
plan. Preserve distinct program `symbol_id` values, mark each affected
function partial with `graph_identity_collision`, and leave calls between the
colliding candidates unresolved. Add a fixture with two traits implementing
the same method name for one type to lock this behavior.

Use these helpers:

```rust
fn span(node: tree_sitter::Node<'_>) -> compass_ir::SourceSpan;
fn text<'a>(source: &'a [u8], node: tree_sitter::Node<'_>) -> &'a str;
fn sha256(bytes: &[u8]) -> String;
fn function_id(
    path: &Path,
    owner: Option<&str>,
    name: &str,
    signature_digest: &str,
) -> String;
fn collect_operations(
    source: &[u8],
    body: tree_sitter::Node<'_>,
) -> (Vec<compass_ir::Operation>, Vec<String>);
```

The seed adapter intentionally emits one basic block per function with a
`Return` or `Unreachable` terminator. Full branch-sensitive CFG construction is
the next Rust deep-analysis subproject. Encountering `if`, `match`, a loop,
`break`, `continue`, or `?` therefore adds an exact partial reason rather than
claiming a complete CFG.

- [ ] **Step 5: Run Rust adapter and existing extraction tests**

Run:

```bash
cargo test -p compass-languages --test program_ir rust_program_ir
cargo test -p compass-languages
cargo clippy -p compass-languages --all-targets -- -D warnings
```

Expected: all commands succeed and existing graph extraction fixtures remain
unchanged.

- [ ] **Step 6: Commit the Rust adapter**

```bash
git add crates/compass-languages
git commit -m "feat(languages): add Rust semantic IR seed adapter"
```

### Task 5: Add the TypeScript and JavaScript seed adapter

**Files:**
- Modify: `crates/compass-languages/src/program/typescript.rs`
- Modify: `crates/compass-languages/tests/program_ir.rs`

**Interfaces:**
- Consumes: the adapter dispatch from Task 4.
- Produces: `ModuleIr` for `.ts`, `.mts`, `.cts`, `.tsx`, `.js`, `.jsx`,
  `.mjs`, and `.cjs` inputs.

- [ ] **Step 1: Add failing TypeScript behavior tests**

Append:

```rust
#[test]
fn typescript_program_ir_captures_methods_callbacks_and_effects()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
export class Checkout {
  async submit(order: Order): Promise<Result> {
    this.pending = order.id;
    const result = await gateway.charge(order);
    return result;
  }
}

export const retry = (job: Job) => queue.enqueue(job);
"#;
    let module = Engine::default()
        .program_ir_source(Path::new("src/checkout.ts"), source)?
        .ok_or("TypeScript adapter returned no module")?;
    assert_eq!(module.language, "typescript");
    assert_eq!(
        module.functions.iter().map(|function| function.name.as_str()).collect::<Vec<_>>(),
        ["Checkout.submit", "retry"]
    );
    let submit = &module.functions[0];
    assert!(submit.blocks[0].operations.iter().any(|operation| matches!(
        &operation.kind,
        OperationKind::Call { callee, receiver_type, .. }
            if callee == "charge" && receiver_type.as_deref() == Some("gateway")
    )));
    assert!(submit.blocks[0].operations.iter().any(|operation| {
        matches!(&operation.kind, OperationKind::Write { path } if path == "this.pending")
    }));
    assert!(submit.blocks[0]
        .operations
        .iter()
        .any(|operation| matches!(operation.kind, OperationKind::Await)));
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify the empty adapter fails**

Run:

```bash
cargo test -p compass-languages --test program_ir typescript_program_ir
```

Expected: the assertion fails because no TypeScript functions are returned.

- [ ] **Step 3: Implement TypeScript-family extraction**

Implement functions, methods, arrow functions bound to variables, calls,
property reads, assignment writes, `await`, and `throw`. Use normalized
repository-relative source-path, class, and function names for stable symbol
IDs. Pre-collect module definitions and resolve only unique same-module calls;
leave imported, dynamic, or ambiguous targets unresolved. Hash exact
signature and body byte ranges. Reproduce the generic structural extractor's
ID inputs for `graph_node_id`; use the adapter contract's
`missing_graph_identity` fallback for constructs the structural tier does not
represent.

Add partial reasons for:

- `dynamic_property_access`
- `prototype_mutation`
- `decorator_semantics`
- `eval_or_function_constructor`
- `branch_sensitive_cfg`
- `exception_flow`

JavaScript functions use `language = "javascript"` and omit unavailable type
references. TSX uses `language = "tsx"` and marks JSX framework dispatch as
`jsx_framework_dispatch`.

- [ ] **Step 4: Test every TypeScript-family registry spelling**

Add a table-driven test using these pairs:

```rust
[
    ("sample.ts", "typescript"),
    ("sample.mts", "typescript"),
    ("sample.cts", "typescript"),
    ("sample.tsx", "tsx"),
    ("sample.js", "javascript"),
    ("sample.jsx", "javascript"),
    ("sample.mjs", "javascript"),
    ("sample.cjs", "javascript"),
]
```

For each file, analyze `const run = () => work();` and assert one function,
one call, the expected module language, and a valid bundle.

- [ ] **Step 5: Run language tests and commit**

Run:

```bash
cargo test -p compass-languages --test program_ir
cargo test -p compass-languages
cargo clippy -p compass-languages --all-targets -- -D warnings
```

Expected: all commands succeed.

```bash
git add crates/compass-languages/src/program/typescript.rs crates/compass-languages/tests/program_ir.rs
git commit -m "feat(languages): add TypeScript semantic IR seed adapter"
```

### Task 6: Integrate program analysis into the atomic graph pipeline

**Files:**
- Modify: `crates/compass-core/Cargo.toml`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-output/src/backup.rs`
- Create: `crates/compass-core/src/program.rs`
- Create: `crates/compass-core/tests/program_pipeline.rs`
- Create: `crates/compass-cli/tests/program_cli.rs`

**Interfaces:**
- Consumes: `Engine::program_ir_source`, the program cache, `compass_analysis::analyze`, `AnalysisBundle::canonical_bytes`, and `write_bytes_atomic`.
- Produces: `program.json`, `BuildResult.program_modules`, `BuildResult.program_summaries`, `BuildResult.program_files_analyzed`, and `BuildResult.program_files_reused`.

- [ ] **Step 1: Write cold, warm, change, and deletion tests**

Create `crates/compass-core/tests/program_pipeline.rs`. The test must:

1. create `src/lib.rs`, `web/app.ts`, and two same-content Rust files at
   different repository-relative paths;
2. set `options.program_analysis = true` and run `build_local_graph`;
3. load `program.json` as `AnalysisBundle`;
4. assert every module source is relative, the same-content files retain
   distinct identities, and every nonempty `graph_node_id` exists in
   `graph.json`;
5. run a warm build and assert every supported file is reused without fresh
   analysis;
6. change only `web/app.ts` and assert one file is analyzed while the other
   supported files are reused;
7. delete `src/lib.rs` and assert no Rust module remains;
8. copy the final source tree into a second temporary checkout with a different
   absolute root, force a clean build there, and compare canonical bytes;
9. replace `program.json` with a directory, change a supported source, and
   assert the build fails while `.compass-build-incomplete` remains; remove the
   obstruction, rerun, and assert the marker is cleared and the artifact is
   valid.

Add focused cases that replace only `adapter_version` or `analyzer_version`
inside an otherwise valid existing artifact. An unchanged build must reject the
stale header, reconstruct from the current cache/analysis code, and restore the
current version without re-running AST extraction.

Use this assertion shape:

```rust
let cold = build_local_graph(&options)?;
assert_eq!(cold.program_files_analyzed, 4);
assert_eq!(cold.program_files_reused, 0);

let warm = build_local_graph(&options)?;
assert_eq!(warm.program_files_analyzed, 0);
assert_eq!(warm.program_files_reused, 4);
```

Create `crates/compass-cli/tests/program_cli.rs`:

```rust
mod support;

use std::error::Error;
use std::fs;
use std::process::Command;

#[test]
fn compass_update_emits_program_artifact_but_graphify_compat_does_not()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let native = directory.path().join("native");
    let compat = directory.path().join("compat");
    fs::create_dir_all(&native)?;
    fs::create_dir_all(&compat)?;
    fs::write(native.join("main.rs"), "fn main() { work(); }\n")?;
    fs::write(compat.join("main.rs"), "fn main() { work(); }\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .arg("update")
        .arg(&native)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(native.join("compass-out/program.json").is_file());

    let output = support::compat_command()
        .arg("update")
        .arg(&compat)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!compat.join("graphify-out/program.json").exists());
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify `BuildResult` lacks program fields**

Run:

```bash
cargo test -p compass-core --test program_pipeline
cargo test -p compass-cli --test program_cli
```

Expected: compilation fails on the missing `BuildResult` fields and native
Compass does not yet create `program.json`.

- [ ] **Step 3: Add program orchestration**

Add dependencies on `compass-ir` and `compass-analysis`. Define in
`program.rs`:

```rust
pub(crate) const PROGRAM_ARTIFACT: &str = "program.json";

pub(crate) struct ProgramBuild {
    pub analysis: compass_analysis::AnalysisBundle,
    pub files_analyzed: usize,
    pub files_reused: usize,
}

pub(crate) fn build_program(
    root: &Path,
    sources: &[PathBuf],
    cache: &mut compass_files::Cache,
    max_workers: usize,
) -> Result<ProgramBuild, CoreError>;

pub(crate) fn write_program(
    output_dir: &Path,
    analysis: &compass_analysis::AnalysisBundle,
) -> Result<(), CoreError>;
```

`write_program` obtains `analysis.canonical_bytes()` and atomically installs
those bytes with `write_bytes_atomic`; it must not independently serialize the
bundle.

`build_program` loads each supported file from:

```rust
CacheKind::Program {
    ir_schema: compass_ir::PROGRAM_SCHEMA_VERSION,
    adapter_version: compass_languages::PROGRAM_ADAPTER_VERSION,
    analysis_schema: compass_analysis::ANALYSIS_SCHEMA_VERSION,
    analyzer_version: compass_analysis::ANALYZER_VERSION,
}
```

Cache values are individual `ModuleIr` records, not a repository bundle. Build
the repository `ProgramBundle` with
`adapter_version: compass_languages::PROGRAM_ADAPTER_VERSION`. Analyze
cache misses with the same sequential-under-256 and bounded Rayon-pool policy as
AST extraction. Before calling `Engine::program_ir_source`, convert each source
to its normalized repository-relative path and pass that logical path with the
already-read bytes. Sort modules through `ProgramBundle::canonicalized`, call
`compass_analysis::analyze`, and save only successful module records. After all
supported files have produced valid cached or fresh modules, call
`prune_program` with exactly those live supported paths. Do not prune after a
failed or incomplete program phase.

- [ ] **Step 4: Wire atomic output and unchanged-build behavior**

Add `BuildOptions.program_analysis: bool` with default `false`. This preserves
the existing behavior of embedders and compatibility entry points until they
opt in. Add these
`BuildResult` fields:

```rust
pub program_modules: usize,
pub program_summaries: usize,
pub program_files_analyzed: usize,
pub program_files_reused: usize,
```

Before the first manifest-unchanged return, if program analysis is enabled,
load and validate the existing `program.json`. That fast path is eligible only
when the artifact's IR schema, adapter version, analysis schema, and analyzer
version all equal the current public constants and its on-disk bytes exactly
equal `AnalysisBundle::canonical_bytes()`. A valid artifact supplies the module
and summary counts without opening the cache. A missing, corrupt, noncanonical,
or version-mismatched artifact disables the return and continues into normal
cache setup.

After AST cache setup, call `build_program` when no valid unchanged artifact was
loaded. Keep the validated `AnalysisBundle` in memory until the graph path has
reached a successful output branch. On every successful clustered, raw,
topology-unchanged, and extract-incremental branch, write `program.json` with
`write_program` immediately before `guard.commit()`. The file
replacement is atomic; if a later artifact write fails, the existing build
guard continues to mark the output incomplete so the next invocation rebuilds
before using the mixed directory.

The later topology-unchanged path uses the freshly reconstructed or previously
validated in-memory artifact. A program artifact failure forces only the
program phase, not AST extraction. A valid artifact loaded by the first fast
path reports its module count as `program_files_reused`, not as a cache hit.

When `program_analysis` is false, remove no user file, emit no artifact, and
report zero program counts.

Update every `BuildResult` constructor in `compass-core` and the
`sample_build_result` fixture in `compass-cli`. At both update/extract and watch
frontend boundaries, set:

```rust
options.program_analysis = frontend == Frontend::Compass;
options.build.program_analysis = frontend == Frontend::Compass;
```

Add unit coverage around `parse_watch_options` asserting native watch enables
the flag and Graphify watch disables it. This keeps every intermediate commit
buildable and prevents update, extract, or watch compatibility frontends from
changing their file sets.

Extend native build and watch status output with one stable line:

```text
Program analysis: <analyzed> analyzed, <reused> reused, <modules> modules, <summaries> summaries
```

Print it only when `program_analysis` is enabled; do not change Graphify
compatibility output. Assert the native line and compatibility omission in
`program_cli.rs`. The qualification script parses this exact line rather than
inferring cache behavior from artifact counts.

- [ ] **Step 5: Make partial analysis visible without failing the graph build**

Add:

```rust
#[error(transparent)]
ProgramAnalysis(#[from] compass_analysis::AnalysisError),
```

Adapter-declared `Completeness::Partial` is valid output. Corrupt cache content
is ignored and recomputed once; if fresh analysis cannot validate, fail the
build before replacing the previous artifact.

- [ ] **Step 6: Preserve the artifact in protected-output backups**

Add `program.json` to `BACKUP_ARTIFACTS` in
`crates/compass-output/src/backup.rs`. Extend its unit test to create a
distinct program fixture before `backup_if_protected`, then assert the dated
backup contains byte-identical `program.json`. This keeps the new authoritative
artifact aligned with the existing graph, analysis, labels, and manifest
recovery contract.

- [ ] **Step 7: Run core pipeline and equivalence tests**

Run:

```bash
cargo test -p compass-core --test program_pipeline
cargo test -p compass-core
cargo test -p compass-output backup
cargo test -p compass-cli --test program_cli
cargo clippy -p compass-core --all-targets -- -D warnings
```

Expected: cold, warm, incremental, deletion, and clean-build outputs are
equivalent and all existing core tests pass.

- [ ] **Step 8: Commit pipeline integration**

```bash
git add crates/compass-core crates/compass-cli/src/lib.rs crates/compass-cli/tests/program_cli.rs crates/compass-output/src/backup.rs
git commit -m "feat(core): emit incremental program analysis"
```

### Task 7: Persist program analysis in graph history

**Files:**
- Modify: `crates/compass-history/Cargo.toml`
- Modify: `crates/compass-history/src/model.rs`
- Modify: `crates/compass-history/src/artifacts.rs`
- Modify: `crates/compass-history/src/keys.rs`
- Modify: `crates/compass-history/src/store.rs`
- Modify: `crates/compass-history/src/validate.rs`
- Modify: `crates/compass-history/src/diff.rs`
- Modify: `crates/compass-history/src/gc.rs`
- Modify: `crates/compass-core/src/history.rs`
- Modify: `crates/compass-cli/src/history_build.rs`
- Modify: `crates/compass-history/tests/roundtrip.rs`
- Modify: `crates/compass-history/tests/publication.rs`
- Create: `crates/compass-history/tests/fixtures/schema2_graph_version.json`
- Modify: `crates/compass-history/tests/performance.rs`
- Modify: `crates/compass-history/tests/diff.rs`
- Modify: `crates/compass-history/tests/maintenance.rs`
- Modify: `crates/compass-core/tests/history_materialize.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Consumes: `program.json` and `compass_analysis::AnalysisBundle`.
- Produces: schema-3 realizations with `program_root`, `program_count`, typed program records, exact program diffs, schema-aware GC and structural sharing, versioned build fingerprints, reconstruction, and schema-2 read compatibility.

- [ ] **Step 1: Add failing artifact round-trip coverage**

Extend the primary round-trip fixture with:

```rust
program: Some(compass_analysis::analyze(compass_ir::ProgramBundle {
    schema_version: compass_ir::PROGRAM_SCHEMA_VERSION,
    adapter_version: 1,
    modules: Vec::new(),
})?),
```

Assert:

```rust
assert_eq!(restored.program, artifacts.program);
assert!(!partitioned.program.is_empty());
```

Add a publication test that changes only one function's
`implementation_digest`, publishes both realizations, and asserts different
realization IDs with a nonempty shared-node count.

Add focused tests that establish:

- full diff returns `RecordKind::Program` for a changed summary;
- topology-only diff excludes program records;
- GC retains schema-3 program roots and does not expect a program root for a
  schema-2 realization;
- changing `PROGRAM_ADAPTER_VERSION` or `ANALYZER_VERSION` changes the
  extraction fingerprint.

- [ ] **Step 2: Run focused tests and verify the fields are absent**

Run:

```bash
cargo test -p compass-history --test roundtrip complete_graph_and_build_state_round_trip
cargo test -p compass-history --test publication publication_is_atomic
```

Expected: compilation fails on missing `program` fields.

- [ ] **Step 3: Add typed program partitioning**

Add `compass-ir` and `compass-analysis` dependencies. Extend:

```rust
pub struct GraphArtifacts {
    pub document: GraphDocument,
    pub analysis: Option<Value>,
    pub labels: Option<Value>,
    pub manifest: Option<Value>,
    pub program: Option<compass_analysis::AnalysisBundle>,
    pub authoritative_sidecars: BTreeMap<String, ArtifactContent>,
}

pub struct PartitionedGraph {
    pub nodes: Vec<(Vec<u8>, Vec<u8>)>,
    pub edges: Vec<(Vec<u8>, Vec<u8>)>,
    pub hyperedges: Vec<(Vec<u8>, Vec<u8>)>,
    pub analysis: Vec<(Vec<u8>, Vec<u8>)>,
    pub program: Vec<(Vec<u8>, Vec<u8>)>,
    pub metadata: Vec<(Vec<u8>, Vec<u8>)>,
}
```

Use typed keys:

```text
[program-schema=1, program-kind=6, "header"]
[program-schema=1, program-kind=6, "module", source_file]
[program-schema=1, program-kind=6, "summary", symbol_id]
[program-schema=1, program-kind=6, "reverse", callee]
```

Store the bundle header with schema versions, store modules and summaries
individually, and store each reverse-caller vector individually. Reconstruct,
validate, canonicalize, and compare the rebuilt bundle with the source.

Register `program.json` as a built-in authoritative artifact with media
type `application/vnd.compass.program+json` and schema version `1`. Load and
write it beside the existing built-in sidecars. Its registry digest and
materialized bytes must come from `AnalysisBundle::canonical_bytes`, never from
a round-trip through `serde_json::Value`.

- [ ] **Step 4: Add schema-3 publication with schema-2 reads**

Set:

```rust
pub const HISTORY_SCHEMA_VERSION: u32 = 3;
pub const LEGACY_HISTORY_SCHEMA_VERSION: u32 = 2;
```

Extend `GraphVersion`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub program_root: Option<StoredTree>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub program_count: Option<u64>,
```

New publications always use schema `3` and set both values, including an empty
program tree. Schema-2 manifests must omit both fields. Accept schema `2` or `3`
while opening:

- schema 2: require both program fields to be absent and preserve its original
  canonical realization ID;
- schema 3: require both program fields to be present and verify the direct
  named root and count;
- every other schema: fail with the existing unsupported-schema diagnostic.

Publish `program` beside `nodes`, `edges`, `hyperedges`, `analysis`, `metadata`,
and `manifest`. Change `publish_catalog_roots` from a fixed six-element array to
a checked slice so schema-2 and schema-3 root sets can coexist.

Audit and update every hard-coded realization-root list in `store.rs`:

- staged publication and destructuring;
- catalog publication;
- direct-root verification;
- full validation;
- artifact reconstruction;
- missing-root diagnostics;
- structural-sharing reachability.

For schema 2, those paths use the existing five data roots plus `manifest`. For
schema 3, they add `program` before `metadata` and `manifest`.

- [ ] **Step 5: Extend validation and reconstruction**

Add `program_records` to `ValidationReport`, make
`RealizationTrees.program: Option<&Tree>`, scan the program tree under the
existing key, value, depth, total-byte, and record-count limits, include it in
`PartitionedGraph`, and let `GraphArtifacts::reconstruct` validate cross-record
consistency.

For schema 2, supply an empty `program` record vector and reconstruct
`GraphArtifacts.program = None`.

- [ ] **Step 6: Include program algorithms in history fingerprints**

In `crates/compass-cli/src/history_build.rs`, add these normalized
build-profile fields in current-profile construction and the supported-field
allowlist:

```text
program_ir_schema=1
program_adapter_version=1
program_analysis_schema=1
program_analyzer_version=1
```

Source the values from the public crate constants rather than repeating
numeric literals in production code.

Make persisted-profile validation accept the enclosing history schema:

- schema 2 requires all four program fields to be absent;
- schema 3 requires all four fields to equal the current public constants;
- any other schema remains unsupported.

Update every `HistoryBuildOptions::from_profile` call to pass the source
realization's schema. When build settings are reused from a schema-2
realization to create new work, preserve its semantic/user options but insert
the four current program fields into the returned profile before building or
publishing; never publish a schema-3 realization with a legacy profile.
Materializing or exporting an already stored schema-2 realization does not
upgrade or rewrite it. Add CLI tests for schema-2 export, schema-2
profile-derived rebuild, missing schema-3 fields, and mismatched schema-3
versions.

In `crates/compass-core/src/history.rs`,
insert the same four fields into `ExtractionFingerprintInput` before digesting
an exact checkout. Add tests that build otherwise identical inputs with one
program version changed and assert different fingerprints.

- [ ] **Step 7: Add exact program diff and schema-aware GC**

Add `RecordKind::Program`. Full `HistoryStore::diff` includes it; topology-only
callers continue requesting only node, edge, and hyperedge roots. Validate and
strip the program typed-key prefix in `display_key`.

When both realizations have program roots, stream their Prolly diff. When
neither has a program root, emit nothing. When exactly one has a program root,
diff it against an empty tree so an explicitly allowed cross-profile comparison
reports added or removed program records instead of hiding them.

Replace `gc.rs`'s fixed `REALIZATION_ROOT_KINDS` assumption with a helper that
derives root kinds from each `GraphVersion`: schema 2 has the existing six
catalog roots; schema 3 has those roots plus `program`. Use the same helper for
prune planning and stale-plan verification.

- [ ] **Step 8: Add a schema-2 golden reopen test**

Before editing `GraphVersion`, use the current schema-2 serializer once to
capture canonical bytes for a fixed, minimal valid set of deterministic test
records in
`tests/fixtures/schema2_graph_version.json` and record its 64-hex
`RealizationId` as a string literal in the test. Review the fixture to confirm
it has no program fields or trailing newline, then do not regenerate it from
schema-3 code. Keep the deterministic record builder in the test so it
recreates the direct-root CIDs named by the fixture.

Publish the fixture's five direct roots and exact manifest bytes through the
existing test Prolly handle, reopen the store, and assert:

```rust
assert_eq!(opened.version.schema_version, 2);
assert!(opened.version.program_root.is_none());
assert!(opened.version.program_count.is_none());
assert_eq!(opened.id.to_string(), EXPECTED_SCHEMA2_REALIZATION_ID);
assert_eq!(
    canonical_json_bytes(&serde_json::to_value(&opened.version)?)?,
    include_bytes!("fixtures/schema2_graph_version.json")
);
```

The expected ID must never be computed from the deserialized fixture inside the
test. This catches a defaulted field, field-order change, or serializer change
that would alter an existing realization's canonical bytes.

- [ ] **Step 9: Run history correctness and performance tests**

Run:

```bash
cargo test -p compass-history --test roundtrip
cargo test -p compass-history --test publication
cargo test -p compass-history --test performance
cargo test -p compass-history --test diff
cargo test -p compass-history --test maintenance
cargo test -p compass-cli --test history_cli
cargo test -p compass-history
cargo clippy -p compass-history --all-targets -- -D warnings
```

Expected: schema-2 stores reopen, schema-3 stores round-trip and diff program
analysis, mixed-schema GC retains the correct root sets, version changes alter
fingerprints, and changing one summary preserves structural sharing for
unchanged program records.

- [ ] **Step 10: Commit history integration**

```bash
git add crates/compass-history crates/compass-core/src/history.rs crates/compass-core/tests/history_materialize.rs crates/compass-cli/src/history_build.rs crates/compass-cli/tests/history_cli.rs
git commit -m "feat(history): version semantic program summaries"
```

### Task 8: Export and qualify the native program artifact

**Files:**
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`
- Modify: `crates/compass-output/src/history_bundle.rs`
- Modify: `crates/compass-output/tests/history_bundle.rs`
- Create: `scripts/qualify_program_foundation.sh`
- Modify: `README.md`
- Modify: `PERFORMANCE.md`

**Interfaces:**
- Consumes: pipeline and history integration from Tasks 6 and 7.
- Produces: lossless `history export --format compass-out`, native output qualification, and a repeatable clean/incremental qualification command.

- [ ] **Step 1: Write failing history-export tests**

Extend `crates/compass-output/tests/history_bundle.rs` with an exact byte
fixture whose field order and escaped non-ASCII identifier make accidental
re-serialization visible:

```rust
#[test]
fn history_bundle_preserves_program_json()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("exported");
    let program = br#"{"schema_version":1,"ir_schema_version":1,"analyzer_version":1,"program":{"schema_version":1,"adapter_version":1,"modules":[]},"summaries":[],"reverse_calls":{"caf\u00e9":[]}}"#;
    let document = document()?;
    let marker = serde_json::json!({
        "schema": "compass.history.completion",
        "schema_version": 1
    });
    let sidecars = std::collections::BTreeMap::new();
    publish_history_bundle(
        &destination,
        &HistoryBundleInput {
            document: &document,
            analysis: None,
            labels: None,
            manifest: None,
            program: Some(program),
            authoritative_sidecars: &sidecars,
            semantic_marker: &marker,
            derived: &[],
        },
    )?;
    assert_eq!(
        std::fs::read(destination.join("program.json"))?,
        program
    );
    Ok(())
}
```

Use the existing `document()` fixture. Add a `history_cli.rs` case that
publishes `GraphArtifacts.program = Some(...)`,
exports `--format compass-out`, and asserts canonical `program.json` equality.
Add a bundle test that supplies an opaque sidecar named `program.json` and
asserts the existing reserved-built-in-path error, proving a caller cannot
shadow the authoritative artifact.

- [ ] **Step 2: Run the tests and verify the bundle input lacks `program`**

Run:

```bash
cargo test -p compass-output --test history_bundle history_bundle_preserves_program_json
cargo test -p compass-cli --test history_cli history_export_preserves_program_json
```

Expected: compilation fails because `HistoryBundleInput.program` is undefined.

- [ ] **Step 3: Wire program data through history export**

Add this field without adding a dependency from `compass-output` to the
analysis crate:

```rust
pub program: Option<&'a [u8]>,
```

`build_staging` writes `program.json` with `write_bytes_atomic`. Staging
validation first parses it as JSON, then compares the complete byte slice.
Add `program.json` to the reserved built-in path list so opaque sidecars cannot
shadow it. Update every `HistoryBundleInput` fixture with `program: None`.

In `history_commands.rs`, obtain the typed bundle's canonical bytes before
constructing the borrowed input:

```rust
let program = artifacts
    .artifacts
    .program
    .as_ref()
    .map(compass_analysis::AnalysisBundle::canonical_bytes)
    .transpose()
    .map_err(runtime)?;
```

Pass `program.as_deref()` to `HistoryBundleInput`. The canonical artifact
registry digest remains owned by `compass-history`; export reproduces the exact
native bytes without regenerating, key-reordering, or reinterpreting program
records.

- [ ] **Step 4: Add a clean-versus-incremental qualification script**

Create an executable `scripts/qualify_program_foundation.sh` that:

1. accepts one repository path;
2. resolves it to a physical absolute path and builds release Compass once;
3. copies the repository into `initial-clean` and `incremental` source
   directories under one temporary directory;
4. writes outputs to separate sibling output roots, never inside either copied
   source tree, and runs a forced clean update in both;
5. creates the same previously absent `compass_qualification_probe.rs` file in
   the incremental copy and runs a non-forced incremental update;
6. creates byte-identical `compass_qualification_probe.rs` in `initial-clean`,
   removes only that checkout's validated temporary output root, and runs a
   forced clean update;
7. compares the two final canonical `program.json` files with `cmp`;
8. asserts the incremental run reports exactly one changed program file and
   that both final artifacts report the same counts;
9. prints module count, summary count, partial count, elapsed time, and artifact
   bytes using a short `python3 -c` JSON reader;
10. removes its temporary directory through a shell trap.

Use `mktemp -d`; reject `/`, `$HOME`, nonexistent input directories, and an
input that already contains `compass_qualification_probe.rs` before copying.
Resolve the release binary to an absolute path before changing directories.
Quote every path, keep the two output roots beneath the validated temporary
directory, and never remove an output path derived from user input.

- [ ] **Step 5: Document the output and support tiers**

Add a README section containing:

```text
compass-out/program.json
```

Document `compass.program/1`, Rust and TypeScript/JavaScript as seed deep-tier
languages, explicit partial reasons, local-only operation, and the distinction
between structural graph facts and deterministic behavior summaries.
Add `program.json` to the authoritative artifact table, history export list,
and any output-cleanup/ownership documentation. Do not mention or accept the
superseded `.compass_program.json` name.

In `PERFORMANCE.md`, document:

```bash
scripts/qualify_program_foundation.sh /path/to/large/repository
```

Record these required fields for each baseline:

- repository commit;
- file, module, function, summary, and partial counts;
- cold and incremental wall time;
- fresh program files analyzed and program files reused;
- peak RSS;
- artifact bytes;
- changed-file count;
- affected-summary count.

- [ ] **Step 6: Run the complete qualification gate**

Run:

```bash
cargo test -p compass-ir
cargo test -p compass-analysis
cargo test -p compass-files
cargo test -p compass-languages
cargo test -p compass-core
cargo test -p compass-history
cargo test -p compass-output --test history_bundle
cargo test -p compass-cli --test program_cli
cargo test -p compass-cli --test history_cli
cargo test -p compass-cli --test update_cli
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/qualify_program_foundation.sh .
compass update .
cd /Users/haipingfu/graphify
graphify update .
git diff --check
```

Expected:

- every command exits `0`;
- native Compass writes `program.json`;
- compatibility mode retains its previous output set;
- warm and incremental analysis report exact fresh and reused program-file counts;
- history export reproduces canonical `program.json`;
- clean and incremental final artifacts are byte-equivalent;
- schema-2 and schema-3 history tests pass;
- `compass-out/` and the Graphify superproject's `graphify-out/` reflect the final code.

- [ ] **Step 7: Commit product qualification**

```bash
git add crates/compass-cli/src/history_commands.rs crates/compass-cli/tests/history_cli.rs crates/compass-output/src/history_bundle.rs crates/compass-output/tests/history_bundle.rs scripts/qualify_program_foundation.sh README.md PERFORMANCE.md
git commit -m "docs: qualify semantic program foundation"
```

## Completion criteria

The foundation is complete only when:

1. `compass update` emits a validated `compass.program/1` artifact for Rust and
   TypeScript-family sources.
2. Unsupported languages remain structurally extracted and do not fabricate
   program analysis.
3. Partial constructs carry exact reasons.
4. Warm and changed-file builds use path-sensitive, algorithm-versioned program
   cache entries and report fresh versus reused files consistently.
5. Incremental and clean final artifacts are byte-equivalent.
6. Behavior summaries and reverse-call indexes are deterministic.
7. Schema-3 history publications preserve program records in their own Prolly
   root.
8. Existing schema-2 realizations reopen with their original realization IDs.
9. Changing one summary reuses unchanged program-tree content.
10. Full history diff includes program records, topology-only diff excludes
    them, mixed-schema GC handles the correct root set, and `compass-out`
    history export restores `program.json`.
11. Graphify compatibility output and existing structural graph behavior remain
    unchanged.
12. Workspace tests, formatting, Clippy, qualification, and graph refresh pass.

## Follow-on implementation plans

After this foundation ships, create separate specs and plans in this order:

1. Rust branch-sensitive CFG and trait/call resolution
2. TypeScript branch-sensitive CFG and framework dispatch
3. Interprocedural data flow, effects, and contract extraction
4. CompassQL program-evidence mapping and witness paths
5. Behavioral diff, impact cones, and test selection
6. Runtime evidence overlays
7. Cross-repository contract federation
8. Agent evidence APIs
