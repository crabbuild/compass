# Semantic IR Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make native Compass builds emit, cache, validate, summarize, and historically preserve a deterministic semantic-program artifact for Rust and TypeScript/JavaScript source files.

**Architecture:** Add a dependency-light `compass-ir` crate for the versioned language-neutral schema and a `compass-analysis` crate for deterministic per-function summaries and reverse dependencies. `compass-languages` supplies seed Rust and TypeScript adapters, `compass-core` orchestrates content-addressed per-file analysis and atomically writes `.compass_program.json`, and `compass-history` stores program records in a sixth Prolly tree while retaining read compatibility with schema-2 realizations.

**Tech Stack:** Rust 1.97.1, Rust 2024 edition, Serde/JSON, SHA-256, tree-sitter 0.26, Rayon, `prolly-map = "=0.5.0"`, `prolly-store-sqlite = "=0.3.0"`, the existing Compass cache, graph pipeline, and history store.

## Global Constraints

- Implement inside the standalone Compass repository at `/Users/haipingfu/graphify/compass`.
- Preserve the workspace's `unsafe_code = "forbid"` and denied Clippy lint policy.
- Structural extraction for every existing language must remain unchanged.
- Rust and TypeScript/JavaScript are the only deep-tier seed adapters in this plan.
- Static facts, derived summaries, runtime observations, hypotheses, and agent assertions remain separate record classes; this plan creates only static facts and deterministic derived summaries.
- Every derived summary carries its source symbol, schema versions, completeness, and deterministic digests.
- Unsupported source files produce no program module; supported but incompletely modeled constructs produce explicit partial reasons.
- Identical source bytes and analyzer versions must produce byte-equivalent canonical program artifacts.
- Incremental output must equal a clean full build.
- Program artifacts use their own versioned Prolly root and share unchanged content across realizations.
- Graphify compatibility mode must not emit `.compass_program.json` or change its legacy output file set.
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
compass-out/.compass_program.json
```

Its schema is `compass.program/1`. It contains normalized per-file IR,
deterministic function summaries, and a reverse-call index. It is authoritative
static analysis state and participates in history realization identity.

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

### History

- `crates/compass-history/src/model.rs`: optional schema-3 program root and count with schema-2 compatibility.
- `crates/compass-history/src/artifacts.rs`: load, partition, reconstruct, register, and write the program artifact.
- `crates/compass-history/src/keys.rs`: typed program keys.
- `crates/compass-history/src/store.rs`: build, publish, open, and verify the program tree.
- `crates/compass-history/src/validate.rs`: scan and validate program records and counts.
- `crates/compass-history/tests/roundtrip.rs`: artifact round trip.
- `crates/compass-history/tests/publication.rs`: schema-3 publication and schema-2 reopen.
- `crates/compass-history/tests/performance.rs`: structural sharing for unchanged summaries.

### Product qualification

- `crates/compass-cli/src/lib.rs`: enable program analysis only for the Compass frontend.
- `crates/compass-cli/tests/program_cli.rs`: native CLI artifact and compatibility isolation.
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

pub struct AnalysisBundle {
    pub schema_version: u32,
    pub ir_schema_version: u32,
    pub program: ProgramBundle,
    pub summaries: Vec<BehaviorSummary>,
    pub reverse_calls: BTreeMap<SymbolId, Vec<SymbolId>>,
}

pub fn analyze(program: ProgramBundle) -> Result<AnalysisBundle, AnalysisError>;
pub fn invalidation_plan(
    previous: &AnalysisBundle,
    current: &AnalysisBundle,
) -> InvalidationPlan;

// compass-languages
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
- Produces: `AnalysisBundle`, `BehaviorSummary`, `EffectKind`, `AnalysisError`, `analyze`, `InvalidationPlan`, and `invalidation_plan`.

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
}
```

`analyze` validates and canonicalizes the program, visits operations in block
and ordinal order, collects call/read/write/effect sets, computes a digest from
the summary with an empty `summary_digest`, sorts summaries by `symbol_id`, and
deduplicates every reverse-call caller list.

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
- Produces: `CacheKind::Program { ir_schema: u32, analysis_schema: u32 }`.

- [ ] **Step 1: Add a failing cache-isolation test**

Append to `crates/compass-files/tests/contracts.rs`:

```rust
#[test]
fn program_cache_is_isolated_by_ir_and_analysis_schema()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("main.rs");
    std::fs::write(&source, "fn main() {}\n")?;
    let mut cache = Cache::new(directory.path(), None)?;
    let v1 = CacheKind::Program {
        ir_schema: 1,
        analysis_schema: 1,
    };
    let v2 = CacheKind::Program {
        ir_schema: 1,
        analysis_schema: 2,
    };
    cache.save(&source, &serde_json::json!({"schema_version": 1}), &v1, None)?;
    assert!(cache.load(&source, &v1, None, false, false)?.is_some());
    assert!(cache.load(&source, &v2, None, false, false)?.is_none());
    assert_ne!(cache.directory(&v1, None), cache.directory(&v2, None));
    cache.clear();
    assert!(cache.load(&source, &v1, None, false, false)?.is_none());
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify the enum variant is absent**

Run:

```bash
cargo test -p compass-files --test contracts program_cache_is_isolated
```

Expected: compilation fails because `CacheKind::Program` is undefined.

- [ ] **Step 3: Implement the program cache namespace**

Extend `CacheKind`:

```rust
Program {
    ir_schema: u32,
    analysis_schema: u32,
},
```

Map it to:

```rust
format!("program/ir{ir_schema}-analysis{analysis_schema}")
```

Update `cached_files` and `clear` to recurse through `cache/program`. Do not
allow semantic prompt fingerprints or AST extractor-version fallback for this
kind. Keep the existing `Ast`, `Semantic`, and `SemanticMode` paths byte-for-byte
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
- Produces: `Engine::program_ir_source` and Rust `ModuleIr` records.

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

- [ ] **Step 4: Implement the Rust seed adapter**

The Rust adapter must:

- create one `FunctionIr` for free functions and `impl` methods;
- use the same `make_id` inputs as `rust_lang.rs` for `graph_node_id`;
- derive `symbol_id` from normalized source path, enclosing type, and function
  name;
- hash the signature range and body range separately with SHA-256;
- preserve parameter order and return-type spelling;
- emit a deterministic entry block `0`;
- emit calls, field reads, assignment writes, `await`, explicit `return`, and
  explicit panic/error macro calls;
- mark `?`, macro-expanded behavior, trait dispatch, and reflection-like calls
  as named partial reasons;
- preserve source order through operation ordinals.

Use these helpers:

```rust
fn span(node: tree_sitter::Node<'_>) -> compass_ir::SourceSpan;
fn text<'a>(source: &'a [u8], node: tree_sitter::Node<'_>) -> &'a str;
fn sha256(bytes: &[u8]) -> String;
fn function_id(path: &Path, owner: Option<&str>, name: &str) -> String;
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
- Produces: `ModuleIr` for `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, and `.cjs` inputs.

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
source-path, class, and function names for stable symbol IDs. Hash exact
signature and body byte ranges.

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
- Create: `crates/compass-core/src/program.rs`
- Create: `crates/compass-core/tests/program_pipeline.rs`

**Interfaces:**
- Consumes: `Engine::program_ir_source`, the program cache, `compass_analysis::analyze`, and `write_json_ascii_atomic`.
- Produces: `.compass_program.json`, `BuildResult.program_modules`, `BuildResult.program_summaries`, and `BuildResult.program_cache_hits`.

- [ ] **Step 1: Write cold, warm, change, and deletion tests**

Create `crates/compass-core/tests/program_pipeline.rs`. The test must:

1. create `src/lib.rs` and `web/app.ts`;
2. run `build_local_graph`;
3. load `.compass_program.json` as `AnalysisBundle`;
4. assert two modules and their summaries;
5. run a warm build and assert two program cache hits;
6. change only `web/app.ts` and assert one program cache hit;
7. delete `src/lib.rs` and assert no Rust module remains;
8. force a clean build in another output directory and compare canonical bytes.

Use this assertion shape:

```rust
let cold = build_local_graph(&options)?;
assert_eq!(cold.program_modules, 2);
assert_eq!(cold.program_summaries, 2);
assert_eq!(cold.program_cache_hits, 0);

let warm = build_local_graph(&options)?;
assert_eq!(warm.program_cache_hits, 2);
```

- [ ] **Step 2: Run the test and verify `BuildResult` lacks program fields**

Run:

```bash
cargo test -p compass-core --test program_pipeline
```

Expected: compilation fails on the missing `BuildResult` fields.

- [ ] **Step 3: Add program orchestration**

Add dependencies on `compass-ir` and `compass-analysis`. Define in
`program.rs`:

```rust
pub(crate) const PROGRAM_ARTIFACT: &str = ".compass_program.json";

pub(crate) struct ProgramBuild {
    pub analysis: compass_analysis::AnalysisBundle,
    pub cache_hits: usize,
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

`build_program` loads each supported file from:

```rust
CacheKind::Program {
    ir_schema: compass_ir::PROGRAM_SCHEMA_VERSION,
    analysis_schema: compass_analysis::ANALYSIS_SCHEMA_VERSION,
}
```

Cache values are individual `ModuleIr` records, not a repository bundle. Analyze
cache misses with the same sequential-under-256 and bounded Rayon-pool policy as
AST extraction. Sort modules through `ProgramBundle::canonicalized`, call
`compass_analysis::analyze`, and save only successful module records.

- [ ] **Step 4: Wire atomic output and unchanged-build behavior**

Add `BuildOptions.program_analysis: bool` with default `true`. Add these
`BuildResult` fields:

```rust
pub program_modules: usize,
pub program_summaries: usize,
pub program_cache_hits: usize,
```

Invoke `build_program` after source detection and AST cache setup but before the
build guard commits. Write `.compass_program.json` through
`write_json_ascii_atomic` before `guard.commit()`.

The unchanged fast path may return only when the program artifact exists and
deserializes under the current IR and analysis versions. A missing, corrupt, or
version-mismatched program artifact forces the program phase without forcing AST
extraction.

When `program_analysis` is false, remove no user file, emit no artifact, and
report zero program counts.

- [ ] **Step 5: Make partial analysis visible without failing the graph build**

Add:

```rust
#[error("invalid program cache for {path}: {detail}")]
InvalidProgramCache { path: PathBuf, detail: String },
#[error("could not serialize program analysis: {0}")]
SerializeProgram(serde_json::Error),
#[error("could not validate program analysis: {0}")]
ProgramAnalysis(String),
```

Adapter-declared `Completeness::Partial` is valid output. Corrupt cache content
is ignored and recomputed once; if fresh analysis cannot validate, fail the
build before replacing the previous artifact.

- [ ] **Step 6: Run core pipeline and equivalence tests**

Run:

```bash
cargo test -p compass-core --test program_pipeline
cargo test -p compass-core
cargo clippy -p compass-core --all-targets -- -D warnings
```

Expected: cold, warm, incremental, deletion, and clean-build outputs are
equivalent and all existing core tests pass.

- [ ] **Step 7: Commit pipeline integration**

```bash
git add crates/compass-core
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
- Modify: `crates/compass-history/tests/roundtrip.rs`
- Modify: `crates/compass-history/tests/publication.rs`
- Modify: `crates/compass-history/tests/performance.rs`
- Modify: `crates/compass-history/tests/diff.rs`
- Modify: `crates/compass-history/tests/maintenance.rs`
- Modify: `crates/compass-core/tests/history_materialize.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Consumes: `.compass_program.json` and `compass_analysis::AnalysisBundle`.
- Produces: schema-3 realizations with `program_root`, `program_count`, typed program records, reconstruction, and schema-2 read compatibility.

- [ ] **Step 1: Add failing artifact round-trip coverage**

Extend the primary round-trip fixture with:

```rust
program: Some(compass_analysis::analyze(compass_ir::ProgramBundle {
    schema_version: compass_ir::PROGRAM_SCHEMA_VERSION,
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

Register `.compass_program.json` as a built-in authoritative artifact with media
type `application/vnd.compass.program+json` and schema version `1`. Load and
write it beside the existing built-in sidecars.

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
and `manifest`.

- [ ] **Step 5: Extend validation and reconstruction**

Add `program_records` to `ValidationReport`, scan the program tree under the
existing key, value, depth, total-byte, and record-count limits, include it in
`PartitionedGraph`, and let `GraphArtifacts::reconstruct` validate cross-record
consistency.

For schema 2, supply an empty `program` record vector and reconstruct
`GraphArtifacts.program = None`.

- [ ] **Step 6: Add a schema-2 golden reopen test**

Construct a schema-2 `GraphVersion` without program fields, serialize it with
canonical JSON, publish its five direct roots and manifest through the existing
test Prolly handle, reopen the store, and assert:

```rust
assert_eq!(opened.version.schema_version, 2);
assert!(opened.version.program_root.is_none());
assert!(opened.version.program_count.is_none());
assert_eq!(opened.id, legacy_id);
```

This test prevents a defaulted field from changing the canonical bytes of an
existing realization.

- [ ] **Step 7: Run history correctness and performance tests**

Run:

```bash
cargo test -p compass-history --test roundtrip
cargo test -p compass-history --test publication
cargo test -p compass-history --test performance
cargo test -p compass-history
cargo clippy -p compass-history --all-targets -- -D warnings
```

Expected: schema-2 stores reopen, schema-3 stores round-trip program analysis,
and changing one summary preserves structural sharing for unchanged program
records.

- [ ] **Step 8: Commit history integration**

```bash
git add crates/compass-history crates/compass-core/tests/history_materialize.rs crates/compass-cli/tests/history_cli.rs
git commit -m "feat(history): version semantic program summaries"
```

### Task 8: Qualify the native CLI contract and document the foundation

**Files:**
- Modify: `crates/compass-cli/src/lib.rs`
- Create: `crates/compass-cli/tests/program_cli.rs`
- Create: `scripts/qualify_program_foundation.sh`
- Modify: `README.md`
- Modify: `PERFORMANCE.md`

**Interfaces:**
- Consumes: pipeline and history integration from Tasks 6 and 7.
- Produces: native Compass output contract, compatibility isolation, and a repeatable qualification command.

- [ ] **Step 1: Write native and compatibility CLI tests**

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
        .args(["update"])
        .arg(&native)
        .output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(native.join("compass-out/.compass_program.json").is_file());

    let output = support::compat_command()
        .arg("update")
        .arg(&compat)
        .output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!compat.join("graphify-out/.compass_program.json").exists());
    Ok(())
}
```

- [ ] **Step 2: Run the test and verify compatibility currently emits the file**

Run:

```bash
cargo test -p compass-cli --test program_cli
```

Expected: the compatibility assertion fails until frontend selection is wired.

- [ ] **Step 3: Select the feature at the frontend boundary**

Immediately after `BuildOptions::new` in the graph-build command:

```rust
options.program_analysis = frontend == Frontend::Compass;
```

History builds use the Compass frontend and retain program analysis. The
internal Graphify compatibility frontend disables it without changing
`COMPASS_OUT`, legacy help, or legacy files.

- [ ] **Step 4: Add a clean-versus-incremental qualification script**

Create an executable `scripts/qualify_program_foundation.sh` that:

1. accepts one repository path;
2. builds release Compass once;
3. copies the repository into two temporary directories;
4. runs a clean update in both;
5. modifies one supported file in the incremental copy;
6. runs an incremental update;
7. force-runs the same final source tree in the clean copy;
8. compares canonical `.compass_program.json` bytes with `cmp`;
9. prints module count, summary count, partial count, elapsed time, and artifact
   bytes using a short `python3 -c` JSON reader;
10. removes its temporary directory through a shell trap.

Use `mktemp -d`; reject `/`, `$HOME`, and nonexistent input directories before
copying.

- [ ] **Step 5: Document the output and support tiers**

Add a README section containing:

```text
compass-out/.compass_program.json
```

Document `compass.program/1`, Rust and TypeScript/JavaScript as seed deep-tier
languages, explicit partial reasons, local-only operation, and the distinction
between structural graph facts and deterministic behavior summaries.

In `PERFORMANCE.md`, document:

```bash
scripts/qualify_program_foundation.sh /path/to/large/repository
```

Record these required fields for each baseline:

- repository commit;
- file, module, function, summary, and partial counts;
- cold and incremental wall time;
- incremental cache hits;
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
cargo test -p compass-cli --test program_cli
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
- native Compass writes `.compass_program.json`;
- compatibility mode retains its previous output set;
- warm and incremental analysis use the program cache;
- clean and incremental final artifacts are byte-equivalent;
- schema-2 and schema-3 history tests pass;
- `compass-out/` and the Graphify superproject's `graphify-out/` reflect the final code.

- [ ] **Step 7: Commit product qualification**

```bash
git add crates/compass-cli/src/lib.rs crates/compass-cli/tests/program_cli.rs scripts/qualify_program_foundation.sh README.md PERFORMANCE.md
git commit -m "docs: qualify semantic program foundation"
```

## Completion criteria

The foundation is complete only when:

1. `compass update` emits a validated `compass.program/1` artifact for Rust and
   TypeScript-family sources.
2. Unsupported languages remain structurally extracted and do not fabricate
   program analysis.
3. Partial constructs carry exact reasons.
4. Warm and changed-file builds use schema-isolated program cache entries.
5. Incremental and clean final artifacts are byte-equivalent.
6. Behavior summaries and reverse-call indexes are deterministic.
7. Schema-3 history publications preserve program records in their own Prolly
   root.
8. Existing schema-2 realizations reopen with their original realization IDs.
9. Changing one summary reuses unchanged program-tree content.
10. Graphify compatibility output and existing structural graph behavior remain
    unchanged.
11. Workspace tests, formatting, Clippy, qualification, and graph refresh pass.

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
