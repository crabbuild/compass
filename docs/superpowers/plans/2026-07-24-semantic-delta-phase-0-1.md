# Semantic Delta Phase 0–1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a deterministic semantic-diff mode to `compass diff` that explains contract, implementation, dependency, and call changes without changing the existing raw graph-diff behavior.

**Architecture:** Fix repository-relative graph identity before comparison, then reconcile the
existing uncommitted `compass-semantic-diff` engine with the approved canonical contract.
`compass-core` owns the shared comparison operation; the CLI only resolves transport arguments,
invokes core, and projects the report. Restore the raw diff implementation removed by concurrent
work and expose semantic review only through `compass diff OLD NEW --semantic`.

**Tech Stack:** Rust 2024, `compass-history` Prolly diffs, `serde`/`serde_json`, SHA-256 fingerprints, existing Compass CLI integration-test harness.

## Global Constraints

- Preserve the current meaning and output schema of raw `compass diff OLD NEW`.
- Semantic mode is opt-in: `compass diff OLD NEW --semantic`.
- All classifications are deterministic and evidence-backed. No model call may affect findings, compatibility, fingerprints, ordering, or exit status.
- Never claim a proven breaking or compatible change from incomplete or ambiguous evidence.
- Normalize identity before diffing; presentation-layer filtering of temporary-worktree churn is not acceptable.
- Treat every path reported by the Task 1 baseline `git status --short` as user-owned.
  Do not restore, reformat, stage, or commit a baseline-dirty path unless a later task names that
  exact path and its before/after diff proves the semantic-delta implementation owns the new hunk.
- In particular, do not overwrite the existing uncommitted multigraph changes in:
  - `crates/compass-graph/src/lib.rs`
  - `crates/compass-graph/tests/build_coverage.rs`
- The repository denies `unwrap`, `expect`, and `panic`; all new production code must comply.
- After every code task, run `graphify update .` from `/Users/haipingfu/graphify/compass`.
- `graphify-out/` and `crates/compass-cli/graphify-out/` are baseline-untracked generated
  directories. Update them as required, but do not stage or commit them in this plan.
- Canonical JSON schema identity is `compass.semantic_delta.report/1`.
- Phase 0–1 intentionally stops at direct semantic findings. Transitive impact, data flow, test selection, CI policy/SARIF, AI narrative, explorer UI, and cross-repository analysis remain later phases.

## Current Worktree Reconciliation

After this plan was drafted, a concurrent implementation based on
`docs/superpowers/plans/2026-07-24-actionable-semantic-diff-mvp.md` appeared in the shared
worktree. It already adds `crates/compass-semantic-diff`, Program IR contract evidence, Git source
deltas, history record reads, semantic renderers, and passing focused tests. It also deletes the
raw CLI diff and makes semantic review the default.

Execution must reuse and review those in-scope changes. Do not create a second delta crate. The
latest approved design takes precedence over that older plan in these conflicts:

- restore raw `compass diff OLD NEW` and its flags and JSON schema;
- require `--semantic` for the semantic report;
- use schema identity `compass.semantic_delta.report/1`;
- keep compatibility, identity strength, evidence strength, and completeness separate;
- route canonical report calculation through `compass-core`, not the CLI adapter;
- retain advanced affected-consumer and test evidence only when it satisfies the same deterministic
  evidence gates; otherwise expose it as a limitation or leave it for a later phase.

Before Task 1, treat the current semantic-diff, CLI, history, IR, language-provider, Cargo, and
lockfile hunks as an uncommitted implementation candidate to audit—not clean baseline code and not
disposable changes.

---

## Task 1: Capture the baseline and protect the dirty worktree

**Files:**

- Inspect: all paths returned by `git status --short`
- Inspect: `Cargo.toml`

- [ ] **Step 1: Record the current worktree and existing user changes**

Run:

```bash
cd /Users/haipingfu/graphify/compass
git status --short
git diff --stat
git diff -- crates/compass-graph/src/lib.rs crates/compass-graph/tests/build_coverage.rs
```

Save the complete path list in the implementation-session notes. These are the baseline-dirty
paths used by every later pre-commit check.

- [ ] **Step 2: Run the focused baseline tests**

Run:

```bash
cargo test -p compass-core pipeline
cargo test -p compass-cli --test history_cli diff_supports_summary_details_streaming_json_and_topology_filtering
cargo test -p compass-cli history_commands::tests
```

Expected: all selected tests pass before semantic-delta changes. If a failure is pre-existing, record the exact command and output before continuing.

- [ ] **Step 3: Commit**

No commit: this task is read-only.

---

## Task 2: Eliminate temporary-worktree identity from AST graph records

**Files:**

- Modify: `crates/compass-core/src/pipeline.rs`
- Test: `crates/compass-core/src/pipeline.rs`

- [ ] **Step 1: Write a failing repository-relative identity test**

Add a test beside `out_of_root_ast_sources_get_portable_ext_ids` that constructs the same logical extraction under two different checkout roots. Include a path-derived phantom import target, because these nodes are not part of `ast_id_remap`.

```rust
#[test]
fn in_root_path_derived_ids_are_stable_across_checkout_roots(
) -> Result<(), Box<dyn Error>> {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let relative_source = Path::new("docs/src/components/Card.astro");
    let relative_target = Path::new("docs/src/consts");

    let build = |root: &Path| {
        let source = root.join(relative_source);
        let target = root.join(relative_target);
        let source_text = source.to_string_lossy().into_owned();
        let target_text = target.to_string_lossy().into_owned();
        let source_id = make_id(&[&source_text]);
        let target_id = make_id(&[&target_text]);
        let mut extraction = Extraction {
            nodes: vec![
                NodeRecord {
                    id: source_id.clone(),
                    attributes: Map::from_iter([
                        ("label".to_owned(), Value::String("Card".to_owned())),
                        ("source_file".to_owned(), Value::String(source_text)),
                    ]),
                },
                NodeRecord {
                    id: target_id.clone(),
                    attributes: Map::from_iter([
                        ("label".to_owned(), Value::String("consts".to_owned())),
                        ("source_file".to_owned(), Value::String(target_text)),
                    ]),
                },
            ],
            edges: vec![EdgeRecord {
                source: source_id,
                target: target_id,
                attributes: Map::from_iter([(
                    "relation".to_owned(),
                    Value::String("imports".to_owned()),
                )]),
            }],
            ..Extraction::default()
        };
        finalize_ast_extraction(&mut extraction, root, &HashMap::new());
        extraction
    };

    let first_graph = build(first.path());
    let second_graph = build(second.path());

    assert_eq!(first_graph.nodes, second_graph.nodes);
    assert_eq!(first_graph.edges, second_graph.edges);
    let encoded = serde_json::to_string(&first_graph)?;
    assert!(!encoded.contains(&first.path().to_string_lossy().into_owned()));
    assert!(!encoded.contains(&second.path().to_string_lossy().into_owned()));
    Ok(())
}
```

- [ ] **Step 2: Verify the test fails for the observed reason**

Run:

```bash
cargo test -p compass-core in_root_path_derived_ids_are_stable_across_checkout_roots -- --exact --nocapture
```

Expected: failure shows different node IDs or edge endpoints derived from each temporary checkout root.

- [ ] **Step 3: Replace external-only remapping with unified path-ID remapping**

In `finalize_ast_extraction`, canonicalize every absolute `source_file`. For a node whose ID is exactly `make_id(&[source])`, map in-root IDs using the same repository-relative identity as detected source nodes, while retaining the existing `ext` namespace for out-of-root sources:

```rust
fn finalize_ast_extraction(
    extraction: &mut Extraction,
    root: &Path,
    ast_id_remap: &HashMap<String, String>,
) {
    apply_ast_id_remap(extraction, ast_id_remap);
    let mut path_id_remap = HashMap::new();
    let mut canonical_sources = HashMap::<String, PathBuf>::new();

    for node in &mut extraction.nodes {
        let Some(source) = node
            .attributes
            .get("source_file")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let source_path = Path::new(&source);
        if !source_path.is_absolute() {
            continue;
        }
        let canonical = canonical_sources
            .entry(source.clone())
            .or_insert_with(|| rooted_source_identity(source_path, root));

        let (portable, canonical_id) = if let Ok(relative) = canonical.strip_prefix(root) {
            let portable = relative.to_string_lossy().replace('\\', "/");
            let canonical_id = make_id(&[&file_stem(relative)]);
            (portable, canonical_id)
        } else {
            let portable = portable_out_of_root_source(source_path, root);
            let canonical_id = make_id(&["ext", &portable]);
            (portable, canonical_id)
        };

        if node.id == make_id(&[&source]) {
            path_id_remap.insert(node.id.clone(), canonical_id);
        }
        node.attributes.insert(
            "source_file".to_owned(),
            serde_json::Value::String(portable),
        );
    }

    if !path_id_remap.is_empty() {
        apply_ast_id_remap(extraction, &path_id_remap);
    }

    for node in &mut extraction.nodes {
        normalize_source_attribute_cached(&mut node.attributes, root, &mut canonical_sources);
        node.attributes.remove("origin_file");
        node.attributes.remove("_callable");
        node.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("ast".to_owned()),
        );
    }
    for edge in &mut extraction.edges {
        normalize_source_attribute_cached(&mut edge.attributes, root, &mut canonical_sources);
        edge.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("ast".to_owned()),
        );
    }
}
```

- [ ] **Step 4: Prove both in-root and out-of-root behavior**

Run:

```bash
cargo test -p compass-core in_root_path_derived_ids_are_stable_across_checkout_roots -- --exact
cargo test -p compass-core out_of_root_ast_sources_get_portable_ext_ids -- --exact
cargo test -p compass-core pipeline
```

Expected: the new two-root regression passes and external paths still use portable `ext` IDs.

- [ ] **Step 5: Update the graph and commit**

Run:

```bash
graphify update .
git add crates/compass-core/src/pipeline.rs
git commit -m "fix: stabilize historical path-derived graph ids"
```

Before committing, inspect `git diff --cached --stat` and confirm neither protected `compass-graph` file is staged.

---

## Task 3: Add the canonical semantic-delta report contract

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/compass-semantic-diff/Cargo.toml`
- Modify: `crates/compass-semantic-diff/src/lib.rs`
- Modify: `crates/compass-semantic-diff/src/model.rs`
- Create: `crates/compass-semantic-diff/tests/report_contract.rs`

- [ ] **Step 1: Write the report-contract test**

Create `crates/compass-semantic-diff/tests/report_contract.rs`:

```rust
use compass_semantic_diff::{
    Compatibility, Comparison, Completeness, EvidenceStrength, IdentityStrength, SemanticDeltaReport,
    SemanticFinding, SemanticKind, Subject, SEMANTIC_DELTA_SCHEMA,
    SEMANTIC_DELTA_SCHEMA_VERSION, SEMANTIC_ENGINE_VERSION, RELATION_REGISTRY_VERSION,
};
use serde_json::json;

#[test]
fn report_serializes_as_versioned_canonical_contract() -> Result<(), Box<dyn std::error::Error>> {
    let report = SemanticDeltaReport::from_findings(
        Comparison {
            old_commit: "1111111111111111111111111111111111111111".to_owned(),
            new_commit: "2222222222222222222222222222222222222222".to_owned(),
            old_realization: "old-realization".to_owned(),
            new_realization: "new-realization".to_owned(),
            old_fingerprint: "a".repeat(64),
            new_fingerprint: "b".repeat(64),
            profile_mismatch: false,
            semantic_engine_version: SEMANTIC_ENGINE_VERSION,
            relation_registry_version: RELATION_REGISTRY_VERSION,
            classifier_versions: std::collections::BTreeMap::from([
                ("contracts/javascript".to_owned(), 1),
                ("contracts/python".to_owned(), 1),
                ("contracts/rust".to_owned(), 1),
                ("contracts/typescript".to_owned(), 1),
                ("graph".to_owned(), 1),
            ]),
            policy_digest: None,
            impact_depth: 0,
        },
        vec![SemanticFinding {
            fingerprint: String::new(),
            kind: SemanticKind::SignatureChanged,
            subject: Subject {
                id: "checkout".to_owned(),
                label: "checkout".to_owned(),
                source_file: Some("src/api.rs".to_owned()),
                symbol_kind: Some("function".to_owned()),
                visibility: Some("public".to_owned()),
                language: Some("rust".to_owned()),
            },
            before: Some(json!({"signature":"checkout(cart)"})),
            after: Some(json!({"signature":"checkout(cart, currency)"})),
            compatibility: Compatibility::PossibleBreak,
            identity: IdentityStrength::Exact,
            evidence_strength: EvidenceStrength::Exact,
            completeness: Completeness::Complete,
            evidence: Vec::new(),
        }],
        Vec::new(),
    )?;

    let value = serde_json::to_value(report)?;
    assert_eq!(value["schema"], SEMANTIC_DELTA_SCHEMA);
    assert_eq!(value["schema_version"], SEMANTIC_DELTA_SCHEMA_VERSION);
    assert_eq!(value["summary"]["findings"], 1);
    assert_eq!(value["summary"]["possible_breaks"], 1);
    assert_eq!(value["findings"][0]["kind"], "signature_changed");
    assert_eq!(value["findings"][0]["compatibility"], "possible_break");
    assert_eq!(value["impacts"], json!([]));
    assert_eq!(value["tests"], json!([]));
    assert_eq!(value["gates"], json!([]));
    assert!(value["narrative"].is_null());
    Ok(())
}
```

- [ ] **Step 2: Verify the current work-in-progress contract fails**

Run:

```bash
cargo test -p compass-semantic-diff --test report_contract
```

Expected: compilation fails because the current work-in-progress exposes
`compass.semantic_diff.report/1`, conflates confidence dimensions, and does not yet provide the
approved `SemanticDeltaReport` contract.

- [ ] **Step 3: Align the existing workspace member and crate manifest**

Keep the concurrent `"crates/compass-semantic-diff"` workspace member. Preserve its existing
`compass-analysis` and `compass-model` dependencies, then ensure this complete dependency set:

```toml
[package]
name = "compass-semantic-diff"
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
compass-history = { path = "../compass-history", version = "0.1.2" }
compass-ir = { path = "../compass-ir", version = "0.1.2" }
compass-analysis = { path = "../compass-analysis", version = "0.1.2" }
compass-model = { path = "../compass-model", version = "0.1.2" }

[lints]
workspace = true
```

- [ ] **Step 4: Implement the public schema**

Define these public types in `model.rs`, all with `Clone`, `Debug`, `Eq`, `PartialEq`, `Serialize`, and `Deserialize` where the contained values support them:

```rust
pub const SEMANTIC_DELTA_SCHEMA: &str = "compass.semantic_delta.report";
pub const SEMANTIC_DELTA_SCHEMA_VERSION: u32 = 1;
pub const SEMANTIC_ENGINE_VERSION: u32 = 1;
pub const RELATION_REGISTRY_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticKind {
    EntityAdded,
    EntityRemoved,
    EntityMoved,
    SignatureChanged,
    ParameterAdded,
    ParameterRemoved,
    ParameterRequiredChanged,
    ParameterTypeChanged,
    ReturnTypeChanged,
    VisibilityChanged,
    ImplementationChanged,
    DependencyAdded,
    DependencyRemoved,
    CallAdded,
    CallRemoved,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    ProvenBreak,
    ProvenCompatible,
    PossibleBreak,
    BehaviorChange,
    NotApplicable,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityStrength {
    Exact,
    Probable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStrength {
    Exact,
    Inferred,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub old_commit: String,
    pub new_commit: String,
    pub old_realization: String,
    pub new_realization: String,
    pub old_fingerprint: String,
    pub new_fingerprint: String,
    pub profile_mismatch: bool,
    pub semantic_engine_version: u32,
    pub relation_registry_version: u32,
    pub classifier_versions: std::collections::BTreeMap<String, u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_digest: Option<String>,
    pub impact_depth: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub record: String,
    pub change: String,
    pub key: Vec<String>,
    pub strength: EvidenceStrength,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticFinding {
    pub fingerprint: String,
    pub kind: SemanticKind,
    pub subject: Subject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
    pub compatibility: Compatibility,
    pub identity: IdentityStrength,
    pub evidence_strength: EvidenceStrength,
    pub completeness: Completeness,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SemanticSummary {
    pub findings: usize,
    pub proven_breaks: usize,
    pub proven_compatible: usize,
    pub possible_breaks: usize,
    pub behavior_changes: usize,
    pub not_applicable: usize,
    pub indeterminate: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangeStory {
    pub fingerprint: String,
    pub title: String,
    pub finding_fingerprints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Limitation {
    pub code: String,
    pub capability: String,
    pub scope: String,
    pub message: String,
    pub remediation: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactOrigin {
    OldGraph,
    NewGraph,
    Combined,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImpactFinding {
    pub fingerprint: String,
    pub source_finding_fingerprint: String,
    pub affected: Subject,
    pub category: String,
    pub origin: ImpactOrigin,
    pub witness: Vec<EvidenceRef>,
    pub distance: u32,
    pub evidence_strength: EvidenceStrength,
    pub completeness: Completeness,
    pub gate_eligible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TestSelection {
    pub test_id: String,
    pub finding_fingerprints: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateState {
    Pass,
    Fail,
    Indeterminate,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GateResult {
    pub policy_id: String,
    pub state: GateState,
    pub finding_fingerprints: Vec<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportProvenance {
    pub deterministic: bool,
    pub model_used: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Narrative {
    pub text: String,
    pub finding_fingerprints: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticDeltaReport {
    pub schema: String,
    pub schema_version: u32,
    pub comparison: Comparison,
    pub summary: SemanticSummary,
    pub stories: Vec<ChangeStory>,
    pub findings: Vec<SemanticFinding>,
    pub impacts: Vec<ImpactFinding>,
    pub tests: Vec<TestSelection>,
    pub gates: Vec<GateResult>,
    pub completeness: std::collections::BTreeMap<String, Completeness>,
    pub provenance: ReportProvenance,
    pub narrative: Option<Narrative>,
    pub limitations: Vec<Limitation>,
}
```

`SemanticDeltaReport::from_findings` must sort findings and limitations deterministically, fill
finding fingerprints, derive the summary and capability-completeness map, and return
`Result<Self, DeltaError>`. Phase 1 emits empty `stories`, `impacts`, `tests`, and `gates`, plus
`narrative: None`; reserving these typed fields now prevents a later silent shape change to the
public `/1` schema. It sets `provenance.deterministic = true` and
`provenance.model_used = false`.

Export all public contract types from `lib.rs`:

```rust
mod fingerprint;
mod model;

pub use model::{
    ChangeStory, Comparison, Compatibility, Completeness, EvidenceRef, EvidenceStrength,
    GateResult, GateState, IdentityStrength, ImpactFinding, ImpactOrigin, Limitation, Narrative,
    ReportProvenance, SemanticDeltaReport, SemanticFinding, SemanticKind, SemanticSummary,
    Subject, TestSelection, RELATION_REGISTRY_VERSION, SEMANTIC_DELTA_SCHEMA,
    SEMANTIC_DELTA_SCHEMA_VERSION, SEMANTIC_ENGINE_VERSION,
};
pub use fingerprint::DeltaError;
```

- [ ] **Step 5: Run the contract test**

Run:

```bash
cargo test -p compass-semantic-diff --test report_contract
cargo clippy -p compass-semantic-diff --all-targets -- -D warnings
```

- [ ] **Step 6: Update the graph and commit**

Run:

```bash
graphify update .
git add Cargo.toml Cargo.lock crates/compass-semantic-diff
git commit -m "feat(delta): define semantic report contract"
```

---

## Task 4: Make findings and reports byte-deterministic

**Files:**

- Create: `crates/compass-semantic-diff/src/fingerprint.rs`
- Modify: `crates/compass-semantic-diff/src/model.rs`
- Create: `crates/compass-semantic-diff/tests/determinism.rs`

- [ ] **Step 1: Write failing order-independence tests**

Test that reversing input findings yields exactly the same serialized report and fingerprints:

```rust
#[test]
fn report_bytes_do_not_depend_on_stream_order() -> Result<(), Box<dyn std::error::Error>> {
    let first = finding(SemanticKind::DependencyAdded, "api", "db");
    let second = finding(SemanticKind::SignatureChanged, "checkout", "checkout(cart, currency)");

    let forward = SemanticDeltaReport::from_findings(
        comparison(),
        vec![first.clone(), second.clone()],
        Vec::new(),
    )?;
    let reverse = SemanticDeltaReport::from_findings(
        comparison(),
        vec![second, first],
        Vec::new(),
    )?;

    assert_eq!(
        serde_json::to_vec(&forward)?,
        serde_json::to_vec(&reverse)?
    );
    assert!(forward.findings.iter().all(|finding| finding.fingerprint.len() == 64));
    assert!(forward.stories.is_empty());
    assert!(forward.impacts.is_empty());
    Ok(())
}
```

Add a second test that constructs two otherwise-identical findings with different display labels,
repository-relative source files, and line metadata. Their fingerprints must match. Then change the
stable subject ID or structured contract value and assert the fingerprint changes.

- [ ] **Step 2: Verify failure**

Run:

```bash
cargo test -p compass-semantic-diff --test determinism
```

Expected: ordering or empty fingerprints cause failure.

- [ ] **Step 3: Implement canonical fingerprinting**

Use field tuples, not arbitrary JSON object order, as fingerprint material:

```rust
pub(crate) fn finding_fingerprint(finding: &SemanticFinding) -> Result<String, DeltaError> {
    digest(&serde_json::to_vec(&(
        SEMANTIC_DELTA_SCHEMA_VERSION,
        SEMANTIC_ENGINE_VERSION,
        classifier_version(finding),
        finding.kind,
        &finding.subject.id,
        &fingerprint_projection(finding.before.as_ref()),
        &fingerprint_projection(finding.after.as_ref()),
        finding.compatibility,
        finding.identity,
        finding.evidence_strength,
        finding.completeness,
    ))?)
}

fn digest(bytes: &[u8]) -> Result<String, DeltaError> {
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
```

Define:

```rust
#[derive(Debug, thiserror::Error)]
pub enum DeltaError {
    #[error("semantic delta serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}
```

Implement `classifier_version` as an exhaustive match returning the version of the owning
classifier family and, for contract findings, the registered `subject.language` rule. Sort findings by
`(subject.source_file, subject.label, kind, subject.id, fingerprint)` and limitations by
`(code, scope, message)`. Display labels and locations determine ordering only; they remain excluded
from fingerprints. `fingerprint_projection` recursively removes display-only keys
`source_file`, `source_location`, `line_start`, `line_end`, `start_byte`, and `end_byte`; contract
fields, stable identities, and repository-relative move destinations represented as
`semantic_path` remain. Phase 2 will add story fingerprints from sorted member-finding
fingerprints.

- [ ] **Step 4: Verify determinism**

Run:

```bash
cargo test -p compass-semantic-diff --test determinism
cargo test -p compass-semantic-diff
cargo clippy -p compass-semantic-diff --all-targets -- -D warnings
```

- [ ] **Step 5: Update the graph and commit**

Run:

```bash
graphify update .
git add crates/compass-semantic-diff
git commit -m "feat(delta): stabilize semantic finding fingerprints"
```

---

## Task 5: Classify entity lifecycle and Program IR contract changes

**Files:**

- Create: `crates/compass-semantic-diff/src/classify.rs`
- Modify: `crates/compass-semantic-diff/src/lib.rs`
- Create: `crates/compass-semantic-diff/tests/node_classification.rs`
- Create: `crates/compass-semantic-diff/tests/program_contracts.rs`

- [ ] **Step 1: Write the classifier matrix as tests**

Cover at least:

| Input | Expected semantic kind | Expected compatibility |
|---|---|---|
| added node | `entity_added` | `not_applicable` |
| removed public node, exact evidence | `entity_removed` | `proven_break` |
| removed private node | `entity_removed` | `not_applicable` |
| public signature hash changed | `signature_changed` | `possible_break` |
| private signature hash changed | `signature_changed` | `indeterminate` |
| public → private visibility | `visibility_changed` | `proven_break` |
| private → public visibility in a registered language | `visibility_changed` | `proven_compatible` |
| implementation hash changed | `implementation_changed` | `behavior_change` |
| only `source_hash` or location changed | no semantic finding | — |
| profile mismatch | same findings plus `profile_mismatch` limitation | never proven |

Use `GraphChange` values that match persisted node records:

```rust
fn node_change(change: ChangeKind, old: Option<Value>, new: Option<Value>) -> GraphChange {
    GraphChange {
        record: RecordKind::Node,
        change,
        key: vec!["checkout".to_owned()],
        old,
        new,
    }
}
```

- [ ] **Step 2: Run and observe failure**

Run:

```bash
cargo test -p compass-semantic-diff --test node_classification
```

- [ ] **Step 3: Write Program IR contract tests**

Persisted `ProgramFact` records with key `["module", "<source-file>"]` contain complete
`compass_ir::ModuleIr` values. Test these cases with two modules whose functions share a stable
`symbol_id`:

| Program IR change on a public function | Expected semantic kind | Exact, complete compatibility |
|---|---|---|
| function removed | `entity_removed` | `proven_break` |
| required parameter added | `parameter_added` | `proven_break` |
| optional parameter added | `parameter_added` | `proven_compatible` |
| required parameter removed | `parameter_removed` | `proven_break` |
| optional parameter removed | `parameter_removed` | `possible_break` |
| optional parameter becomes required | `parameter_required_changed` | `proven_break` |
| required parameter becomes optional | `parameter_required_changed` | `proven_compatible` |
| parameter type spelling/resolution changes | `parameter_type_changed` | `possible_break` |
| return type spelling/resolution changes | `return_type_changed` | `possible_break` |
| signature digest changes with no more precise field difference | `signature_changed` | `possible_break` |
| body digest changes | `implementation_changed` | `behavior_change` |

Also assert:

- private-function contract changes are `indeterminate`, not proven breaks;
- an otherwise-proven change in an unregistered language is `possible_break` or
  `indeterminate`, never proven;
- partial/indeterminate `Capability::Contracts` coverage prevents proven conclusions;
- partial/indeterminate `Capability::SymbolIdentity` yields `IdentityStrength::Probable`;
- a `profile_mismatch` limitation downgrades every otherwise-proven result to `indeterminate`;
- a key or value containing `git_compass_tmp_worktree`, `.git/compass/tmp/worktree`, or an
  absolute `source_file` creates one `unstable_identity` limitation, marks identity completeness
  partial, and produces no finding from that raw record;
- a matching Program IR function contract suppresses the duplicate coarse graph-node
  `signature_changed` finding for its `graph_node_id`;
- an exact move keyed by an unchanged `scip_symbol` emits one `entity_moved` finding rather than
  independent removal and addition.

Run:

```bash
cargo test -p compass-semantic-diff --test program_contracts
```

- [ ] **Step 4: Implement a collecting builder and Program IR-first reconciliation**

Public API:

```rust
pub struct SemanticDeltaBuilder {
    comparison: Comparison,
    changes: Vec<GraphChange>,
    limitations: Vec<Limitation>,
}

impl SemanticDeltaBuilder {
    #[must_use]
    pub fn new(comparison: Comparison) -> Self;

    pub fn push(&mut self, change: &GraphChange);

    pub fn finish(self) -> Result<SemanticDeltaReport, DeltaError>;
}
```

`push` is intentionally infallible and stores each streamed change. `finish` classifies changed
Program IR modules first, records their matched `graph_node_id` values, and then classifies graph
nodes and edges. That ordering prevents a precise parameter change and its coarse node
`signature_hash` change from becoming duplicate findings. Malformed or insufficient values create
an `indeterminate` finding or structured limitation rather than aborting the complete comparison.

Before storing a change, recursively validate its key, old value, and new value. A known Compass
temporary-worktree marker anywhere in stable identity material, or an absolute `source_file`,
produces one deduplicated `unstable_identity` limitation and excludes that record from findings.
Do not attempt to strip the prefix inside the delta engine: Phase 0 fixes producers, while this
guard prevents older corrupt realizations from supporting a false compatibility claim.

Program-record selection and typed decoding:

```rust
fn changed_module(change: &GraphChange) -> Option<ChangedModule> {
    if change.record != RecordKind::ProgramFact
        || change.key.first().map(String::as_str) != Some("module")
    {
        return None;
    }
    Some(ChangedModule {
        old: change
            .old
            .clone()
            .and_then(|value| serde_json::from_value::<compass_ir::ModuleIr>(value).ok()),
        new: change
            .new
            .clone()
            .and_then(|value| serde_json::from_value::<compass_ir::ModuleIr>(value).ok()),
        evidence: evidence_ref(change, EvidenceStrength::Exact),
    })
}
```

Do not silently discard a decode failure in production: `finish` must add a
`program_module_decode_failed` limitation for any present old/new module value that does not
deserialize.

Match functions by stable `symbol_id`, and parameters by `(kind, name)` within a matched function.
Treat a parameter rename as one removal plus one addition. Compare `TypeRef` by
`(spelling, resolved_symbol)`.

Implement a versioned language-contract registry for the Program IR providers present in this
workspace (`rust`, `python`, `typescript`, and `javascript`). A proven result requires an explicit
rule for the module language; unknown language strings are conservative. Record each registry
version in `comparison.classifier_versions` as `contracts/<language>`. Do not apply a generic
cross-language proof merely because two serialized parameter arrays differ.

Before ordinary node lifecycle classification, align removed and added SCIP-backed nodes whose
`metadata.scip_symbol` values are identical and non-empty. Emit `EntityMoved` with exact identity
and repository-relative old/new locations, then mark both raw records consumed. Do not infer a
probable rename from labels, paths, or matching hashes in Phase 1.

Derive evidence gates from both old and new function/module coverage maps:

```rust
fn capability_completeness(
    old: &compass_ir::Coverage,
    new: &compass_ir::Coverage,
    capability: compass_ir::Capability,
) -> Completeness {
    use compass_ir::CoverageState;
    match (old.get(&capability), new.get(&capability)) {
        (Some(CoverageState::Complete), Some(CoverageState::Complete)) => {
            Completeness::Complete
        }
        (Some(CoverageState::Failed { .. }), _)
        | (_, Some(CoverageState::Failed { .. }))
        | (None, _)
        | (_, None) => Completeness::Unavailable,
        _ => Completeness::Partial,
    }
}
```

Merge module and function coverage conservatively: a capability is complete only when both levels
that declare it are complete; any declared partial/indeterminate/failed state downgrades it.

Extract only a small semantic projection from old/new records:

```rust
fn subject(value: &Value, fallback_id: &str) -> Subject {
    Subject {
        id: string(value, "id").unwrap_or_else(|| fallback_id.to_owned()),
        label: string(value, "label").unwrap_or_else(|| fallback_id.to_owned()),
        source_file: string(value, "source_file"),
        symbol_kind: string(value, "symbol_kind"),
        visibility: string(value, "visibility"),
        language: string(value, "language"),
    }
}

fn contract_projection(value: &Value) -> Value {
    json!({
        "signature": value.get("signature"),
        "signature_hash": value.get("signature_hash"),
        "visibility": value.get("visibility"),
    })
}

fn implementation_projection(value: &Value) -> Value {
    json!({
        "implementation_hash": value.get("implementation_hash"),
    })
}
```

Rules:

- Added/removed records use the present side as the subject.
- A public or externally visible removed entity is `ProvenBreak` only when identity and evidence are exact and the comparison profile matches; otherwise downgrade to `PossibleBreak` or `Indeterminate`.
- When Program IR emits a precise lifecycle, contract, visibility, or implementation finding for
  a function, suppress all coarse node findings for its `graph_node_id`, not just signature
  findings.
- Use Program IR parameters and return types when available. A public required parameter addition
  or optional-to-required change is proven breaking only with exact symbol identity, exact evidence,
  complete `Contracts` coverage, and comparable profiles.
- A coarse graph-node signature change is not a proven break because a hash alone does not prove
  which language-level contract changed. Add one deduplicated `coarse_signature_only` limitation
  when any such finding is emitted.
- Visibility narrowing from a known public visibility to a known non-public visibility is a proven break under exact, complete evidence.
- Program IR `body_digest` or graph-node `implementation_hash` changes are `BehaviorChange`;
  source-hash-only changes are omitted.
- If `comparison.profile_mismatch`, add one limitation and downgrade every `ProvenBreak`/`ProvenCompatible` result to `Indeterminate`.

- [ ] **Step 5: Run the matrices**

Run:

```bash
cargo test -p compass-semantic-diff --test node_classification
cargo test -p compass-semantic-diff --test program_contracts
cargo test -p compass-semantic-diff
cargo clippy -p compass-semantic-diff --all-targets -- -D warnings
```

- [ ] **Step 6: Update the graph and commit**

Run:

```bash
graphify update .
git add crates/compass-semantic-diff
git commit -m "feat(delta): classify semantic contract changes"
```

---

## Task 6: Classify dependency and call relationship changes

**Files:**

- Modify: `crates/compass-semantic-diff/src/classify.rs`
- Create: `crates/compass-semantic-diff/tests/edge_classification.rs`

- [ ] **Step 1: Write relationship tests**

Cover:

- `imports` and `imports_from` added/removed → dependency findings.
- `calls` and `indirect_call` added/removed → call findings.
- `confidence: "EXTRACTED"` → exact evidence.
- `confidence: "INFERRED"` → inferred evidence and no proven compatibility.
- ambiguous/unresolved endpoint → ambiguous evidence and `Indeterminate`.
- unrelated graph relationships are omitted in Phase 1.
- a changed edge attribute does not become a false add/remove.

Example:

```rust
#[test]
fn removed_extracted_import_is_a_dependency_change() -> Result<(), DeltaError> {
    let mut builder = SemanticDeltaBuilder::new(comparison());
    builder.push(&GraphChange {
        record: RecordKind::Edge,
        change: ChangeKind::Removed,
        key: vec![
            "api".to_owned(),
            "database".to_owned(),
            "imports".to_owned(),
            String::new(),
        ],
        old: Some(json!({
            "source": "api",
            "target": "database",
            "relation": "imports",
            "confidence": "EXTRACTED",
            "source_file": "src/api.rs"
        })),
        new: None,
    });
    let report = builder.finish()?;
    assert_eq!(report.findings[0].kind, SemanticKind::DependencyRemoved);
    assert_eq!(report.findings[0].evidence_strength, EvidenceStrength::Exact);
    assert_eq!(report.findings[0].compatibility, Compatibility::BehaviorChange);
    Ok(())
}
```

- [ ] **Step 2: Verify failure**

Run:

```bash
cargo test -p compass-semantic-diff --test edge_classification
```

- [ ] **Step 3: Implement edge classification**

Build a stable edge subject:

```rust
fn edge_subject(value: &Value, key: &[String]) -> Subject {
    let source = string(value, "source")
        .or_else(|| key.first().cloned())
        .unwrap_or_else(|| "<unknown-source>".to_owned());
    let target = string(value, "target")
        .or_else(|| key.get(1).cloned())
        .unwrap_or_else(|| "<unknown-target>".to_owned());
    let relation = string(value, "relation")
        .or_else(|| key.get(2).cloned())
        .unwrap_or_else(|| "<unknown-relation>".to_owned());
    Subject {
        id: format!("{source}:{relation}:{target}"),
        label: format!("{source} {relation} {target}"),
        source_file: string(value, "source_file"),
        symbol_kind: Some("relationship".to_owned()),
        visibility: None,
        language: string(value, "language"),
    }
}
```

Use `BehaviorChange` for exact dependency/call additions and removals: the direct relation changed, but Phase 1 does not prove API compatibility. Use `Indeterminate` when endpoints or evidence are ambiguous.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p compass-semantic-diff --test edge_classification
cargo test -p compass-semantic-diff
cargo clippy -p compass-semantic-diff --all-targets -- -D warnings
```

- [ ] **Step 5: Update the graph and commit**

Run:

```bash
graphify update .
git add crates/compass-semantic-diff
git commit -m "feat(delta): classify dependency and call changes"
```

---

## Task 7: Add the shared core semantic-diff operation

**Files:**

- Modify: `crates/compass-core/Cargo.toml`
- Create: `crates/compass-core/src/semantic_diff.rs`
- Modify: `crates/compass-core/src/lib.rs`
- Create: `crates/compass-core/tests/semantic_diff.rs`

- [ ] **Step 1: Write a failing core integration test**

Publish two tiny immutable graph versions through `HistoryStore`, invoke the core operation, and assert it returns a contract finding plus comparison identity:

```rust
let report = semantic_diff(SemanticDiffRequest {
    history: &history,
    old: &old,
    new: &new,
    profile_mismatch: false,
})?;

assert_eq!(report.comparison.old_commit, old.version.git_commit.to_string());
assert_eq!(report.comparison.new_commit, new.version.git_commit.to_string());
assert!(report
    .findings
    .iter()
    .any(|finding| finding.kind == SemanticKind::SignatureChanged));
```

Add a second test where raw history contains only analysis/metadata changes and confirm the
operation returns no semantic finding. Add a third with a changed Program IR module and assert a
required-parameter addition is classified.

- [ ] **Step 2: Verify failure**

Run:

```bash
cargo test -p compass-core --test semantic_diff
```

- [ ] **Step 3: Implement the operation**

Add `compass-semantic-diff` to `compass-core` dependencies and implement:

```rust
use compass_semantic_diff::{Comparison, DeltaError, SemanticDeltaBuilder, SemanticDeltaReport};
use compass_history::{
    ChangeSink, GraphChange, HistoryError, HistoryStore, PublishedVersion, RecordKind,
};

pub struct SemanticDiffRequest<'a> {
    pub history: &'a HistoryStore,
    pub old: &'a PublishedVersion,
    pub new: &'a PublishedVersion,
    pub profile_mismatch: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SemanticDiffError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Delta(#[from] DeltaError),
}

struct DeltaSink {
    builder: SemanticDeltaBuilder,
}

impl ChangeSink for DeltaSink {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        self.builder.push(&change);
        Ok(())
    }
}

pub fn semantic_diff(
    request: SemanticDiffRequest<'_>,
) -> Result<SemanticDeltaReport, SemanticDiffError> {
    let comparison = Comparison {
        old_commit: request.old.version.git_commit.to_string(),
        new_commit: request.new.version.git_commit.to_string(),
        old_realization: request.old.id.to_string(),
        new_realization: request.new.id.to_string(),
        old_fingerprint: request.old.version.extraction_fingerprint.to_string(),
        new_fingerprint: request.new.version.extraction_fingerprint.to_string(),
        profile_mismatch: request.profile_mismatch,
        semantic_engine_version: compass_semantic_diff::SEMANTIC_ENGINE_VERSION,
        relation_registry_version: compass_semantic_diff::RELATION_REGISTRY_VERSION,
        classifier_versions: std::collections::BTreeMap::from([
            ("contracts/javascript".to_owned(), 1),
            ("contracts/python".to_owned(), 1),
            ("contracts/rust".to_owned(), 1),
            ("contracts/typescript".to_owned(), 1),
            ("graph".to_owned(), 1),
        ]),
        policy_digest: None,
        impact_depth: 0,
    };
    let mut sink = DeltaSink {
        builder: SemanticDeltaBuilder::new(comparison),
    };
    request.history.diff_records(
        &request.old.id,
        &request.new.id,
        &[
            RecordKind::Node,
            RecordKind::Edge,
            RecordKind::ProgramFact,
        ],
        &mut sink,
    )?;
    Ok(sink.builder.finish()?)
}
```

Re-export `semantic_diff`, `SemanticDiffError`, and `SemanticDiffRequest` from `compass-core/src/lib.rs`.

Revision parsing and materialization remain in the CLI during Phase 1 because they already share the history builder and profile-selection path. This does not put semantic interpretation in the adapter: the CLI resolves transport inputs, while core owns the full report calculation.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p compass-core --test semantic_diff
cargo test -p compass-core
cargo clippy -p compass-core --all-targets -- -D warnings
```

- [ ] **Step 5: Update the graph and commit**

Run:

```bash
graphify update .
git add crates/compass-core/Cargo.toml crates/compass-core/src/lib.rs \
  crates/compass-core/src/semantic_diff.rs crates/compass-core/tests/semantic_diff.rs \
  Cargo.lock
git commit -m "feat(core): expose semantic history comparison"
```

---

## Task 8: Add `--semantic` parsing without changing raw diff

**Files:**

- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Test: `crates/compass-cli/src/history_commands.rs`

- [ ] **Step 1: Add failing parser tests**

Add tests for:

```rust
#[test]
fn parses_semantic_diff() -> Result<(), String> {
    let (_, options) = parse_diff(&[
        "OLD".to_owned(),
        "NEW".to_owned(),
        "--semantic".to_owned(),
    ])?;
    assert!(options.semantic);
    assert_eq!(options.output, DiffOutput::Summary);
    Ok(())
}

#[test]
fn semantic_diff_accepts_json() -> Result<(), String> {
    let (_, options) = parse_diff(&[
        "OLD".to_owned(),
        "NEW".to_owned(),
        "--semantic".to_owned(),
        "--format=json".to_owned(),
    ])?;
    assert!(options.semantic);
    assert_eq!(options.output, DiffOutput::Json);
    Ok(())
}

#[test]
fn semantic_diff_rejects_raw_projection_flags() -> Result<(), String> {
    for flag in [
        "--detailed",
        "--topology-only",
        "--include-locations",
        "--include-analysis",
        "--include-metadata",
    ] {
        let arguments = ["OLD", "NEW", "--semantic", flag]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let error = match parse_diff(&arguments) {
            Ok(_) => return Err(format!("expected --semantic to conflict with {flag}")),
            Err(error) => error,
        };
        assert!(error.contains("--semantic"));
        assert!(error.contains(flag));
    }
    Ok(())
}
```

- [ ] **Step 2: Verify failure**

Run:

```bash
cargo test -p compass-cli history_commands::tests::parses_semantic_diff
```

- [ ] **Step 3: Implement parsing**

Add `semantic: bool` to `DiffOptions`, parse `--semantic` with duplicate detection, and reject the raw-only projection flags after parsing:

```rust
if semantic {
    for (enabled, flag) in [
        (detailed, "--detailed"),
        (topology_only, "--topology-only"),
        (include_locations, "--include-locations"),
        (include_analysis, "--include-analysis"),
        (include_metadata, "--include-metadata"),
    ] {
        if enabled {
            return Err(format!("--semantic cannot be combined with {flag}"));
        }
    }
}
```

Keep `--fingerprint` and `--allow-profile-mismatch` supported because they select or qualify the compared evidence rather than alter projection.

- [ ] **Step 4: Update both help surfaces**

Change `diff_help` and the `help.rs` diff page to include:

```text
--semantic                     Explain deterministic semantic and compatibility changes
```

Add examples:

```text
compass diff v1.2.0 HEAD --semantic
compass diff main feature --semantic --format json
```

State that semantic mode conflicts with raw projection flags and that raw diff remains the default.

- [ ] **Step 5: Verify parsing and help**

Run:

```bash
cargo test -p compass-cli history_commands::tests
cargo test -p compass-cli help
```

- [ ] **Step 6: Update the graph and commit**

Run:

```bash
graphify update .
git add crates/compass-cli/src/history_commands.rs crates/compass-cli/src/help.rs
git commit -m "feat(cli): parse semantic diff mode"
```

---

## Task 9: Render reviewer text and canonical JSON

**Files:**

- Modify: `crates/compass-cli/Cargo.toml`
- Create: `crates/compass-cli/src/semantic_diff.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

- [ ] **Step 1: Write failing CLI integration tests**

Publish an old and new graph with:

- a public function signature change;
- an `implementation_hash` change;
- one removed import;
- one added call.

Assert text includes this structure:

```text
Semantic changes: 4
Compatibility: 0 proven breaks, 0 proven compatible, 1 possible break, 3 behavior changes

What changed
  POSSIBLE BREAK  checkout — signature changed
    src/api.rs
  BEHAVIOR CHANGE api imports database — dependency removed

What may break
  checkout: public signature changed; language-level substitutability is not proven

Affected
  src/api.rs: api imports database, checkout

Limitations
  Compatibility for signature changes is conservative in this phase.
```

Assert JSON:

```rust
let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
assert_eq!(value["schema"], "compass.semantic_delta.report");
assert_eq!(value["schema_version"], 1);
assert_eq!(value["findings"][0]["fingerprint"].as_str().map(str::len), Some(64));
assert!(value["findings"].as_array().is_some_and(|findings| !findings.is_empty()));
```

Also run the same raw diff fixture without `--semantic` and assert its existing schema remains `schema_version: 2` with a `changes` array.

- [ ] **Step 2: Verify failure**

Run:

```bash
cargo test -p compass-cli --test history_cli semantic_diff_renders_reviewer_text_and_json
```

- [ ] **Step 3: Implement the text projection**

Add `compass-semantic-diff` as a direct CLI dependency and create:

```rust
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use compass_semantic_diff::{Compatibility, SemanticDeltaReport};
use compass_history::HistoryError;

pub(crate) fn render_text(
    report: &SemanticDeltaReport,
    writer: &mut dyn Write,
) -> Result<(), HistoryError> {
    if report.findings.is_empty() {
        writer
            .write_all(b"no semantic changes\n")
            .map_err(output_error)?;
        return render_limitations(report, writer);
    }

    writeln!(writer, "Semantic changes: {}", report.summary.findings)
        .map_err(output_error)?;
    writeln!(
        writer,
        "Compatibility: {} proven breaks, {} proven compatible, {} possible breaks, {} behavior changes, {} indeterminate",
        report.summary.proven_breaks,
        report.summary.proven_compatible,
        report.summary.possible_breaks,
        report.summary.behavior_changes,
        report.summary.indeterminate,
    )
    .map_err(output_error)?;
    writeln!(writer, "\nWhat changed").map_err(output_error)?;

    for finding in &report.findings {
        writeln!(
            writer,
            "  {:<15} {} — {}",
            compatibility_label(finding.compatibility),
            finding.subject.label,
            kind_label(finding.kind),
        )
        .map_err(output_error)?;
        if let Some(source_file) = &finding.subject.source_file {
            writeln!(writer, "    {source_file}").map_err(output_error)?;
        }
    }

    let risky = report
        .findings
        .iter()
        .filter(|finding| matches!(
            finding.compatibility,
            Compatibility::ProvenBreak
                | Compatibility::PossibleBreak
                | Compatibility::Indeterminate
        ))
        .collect::<Vec<_>>();
    if !risky.is_empty() {
        writeln!(writer, "\nWhat may break").map_err(output_error)?;
        for finding in risky {
            writeln!(
                writer,
                "  {}: {}",
                finding.subject.label,
                risk_explanation(finding),
            )
            .map_err(output_error)?;
        }
    }

    let mut affected = BTreeMap::<&str, BTreeSet<&str>>::new();
    for finding in &report.findings {
        let scope = finding
            .subject
            .source_file
            .as_deref()
            .unwrap_or("<repository>");
        affected
            .entry(scope)
            .or_default()
            .insert(&finding.subject.label);
    }
    writeln!(writer, "\nAffected").map_err(output_error)?;
    for (scope, labels) in affected {
        writeln!(
            writer,
            "  {scope}: {}",
            labels.into_iter().collect::<Vec<_>>().join(", "),
        )
        .map_err(output_error)?;
    }

    render_limitations(report, writer)
}

fn render_limitations(
    report: &SemanticDeltaReport,
    writer: &mut dyn Write,
) -> Result<(), HistoryError> {
    if report.limitations.is_empty() {
        return Ok(());
    }
    writeln!(writer, "\nLimitations").map_err(output_error)?;
    for limitation in &report.limitations {
        writeln!(writer, "  {}", limitation.message).map_err(output_error)?;
    }
    Ok(())
}

fn output_error(source: std::io::Error) -> HistoryError {
    HistoryError::Io {
        path: std::path::PathBuf::from("<stdout>"),
        source,
    }
}
```

Keep all labels static and deterministic. Do not infer prose from fields the canonical report does not contain.
Implement exhaustive `compatibility_label`, `kind_label`, and `risk_explanation` matches in the
same module; do not expose debug formatting as public output.

- [ ] **Step 4: Route semantic mode through core**

At the top of `render_diff`, branch on `options.semantic`:

```rust
if options.semantic {
    let report = compass_core::semantic_diff(compass_core::SemanticDiffRequest {
        history,
        old,
        new,
        profile_mismatch,
    })
    .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
    return match options.output {
        DiffOutput::Json => {
            serde_json::to_writer(&mut *writer, &report).map_err(json_output_error)?;
            writer.write_all(b"\n").map_err(output_error)
        }
        DiffOutput::Summary => semantic_diff::render_text(&report, writer),
        DiffOutput::Detailed => Err(HistoryError::InvalidArtifacts(
            "--semantic cannot use detailed raw output".to_owned(),
        )),
    };
}
```

Register `mod semantic_diff;` in `lib.rs`. Keep the existing raw match below this branch byte-for-byte except for formatting required by `cargo fmt`.

For profile mismatch, semantic JSON carries the structured limitation and `comparison.profile_mismatch`; text still emits the existing stderr warning plus the report limitation.

- [ ] **Step 5: Verify semantic and raw output**

Run:

```bash
cargo test -p compass-cli --test history_cli semantic_diff_renders_reviewer_text_and_json
cargo test -p compass-cli --test history_cli diff_supports_summary_details_streaming_json_and_topology_filtering
cargo test -p compass-cli history_commands::tests
cargo clippy -p compass-cli --all-targets -- -D warnings
```

- [ ] **Step 6: Update the graph and commit**

Run:

```bash
graphify update .
git add crates/compass-cli/Cargo.toml crates/compass-cli/src/lib.rs \
  crates/compass-cli/src/history_commands.rs crates/compass-cli/src/semantic_diff.rs \
  crates/compass-cli/tests/history_cli.rs Cargo.lock
git commit -m "feat(cli): render semantic history differences"
```

---

## Task 10: Prove historical replay stability and document the command

**Files:**

- Modify: `crates/compass-cli/tests/history_cli.rs`
- Modify: `README.md`

- [ ] **Step 1: Add a historical replay acceptance test**

Create a temporary Git repository with two commits containing an Astro component that imports an extensionless repository path. Run the normal Compass history build/rebuild flow twice for the same commit. Export both realizations or compare their stored node/edge roots and assert:

```rust
assert_eq!(first.version.nodes_root, second.version.nodes_root);
assert_eq!(first.version.edges_root, second.version.edges_root);
```

Then run semantic diff between the two logical source commits and assert the report contains no finding whose subject or evidence contains:

```text
.git_compass_tmp_worktree
git_compass_tmp_worktree
compass/tmp/worktree
```

The test must exercise the production materializer, not call `finalize_ast_extraction` directly.

- [ ] **Step 2: Run the acceptance test**

Run:

```bash
cargo test -p compass-cli --test history_cli historical_replay_uses_stable_repository_identity -- --exact --nocapture
```

- [ ] **Step 3: Document user commands and interpretation**

Add a concise README section:

````markdown
### Explain semantic changes between revisions

```bash
compass diff v1.2.0 HEAD --semantic
compass diff main feature --semantic --format json
```

Semantic mode reports deterministic contract, implementation, dependency, and call changes.
`proven_break` is reserved for conclusions supported by exact and complete evidence;
`possible_break` and `indeterminate` mean reviewer judgment is still required.
Use raw `compass diff OLD NEW --detailed` when you need underlying graph-record changes.
````

- [ ] **Step 4: Run complete verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-semantic-diff
cargo test -p compass-core
cargo test -p compass-cli
cargo clippy -p compass-semantic-diff -p compass-core -p compass-cli --all-targets -- -D warnings
graphify update .
git diff --check
```

Then verify the protected pre-existing changes are still present and were not included in any semantic-delta commit:

```bash
git status --short
git diff -- crates/compass-graph/src/lib.rs crates/compass-graph/tests/build_coverage.rs
git log --stat --oneline HEAD~10..HEAD
```

- [ ] **Step 5: Run a real-repository smoke test**

From `<qualification-corpus-root>/cocoindex`, using the newly built binary:

```bash
/Users/haipingfu/graphify/compass/target/debug/compass history build HEAD~1 --code-only
/Users/haipingfu/graphify/compass/target/debug/compass history build HEAD --code-only
/Users/haipingfu/graphify/compass/target/debug/compass diff HEAD~1 HEAD --semantic
/Users/haipingfu/graphify/compass/target/debug/compass diff HEAD~1 HEAD --semantic --format json
```

Check that:

- no subject/evidence ID contains a Compass temporary-worktree path;
- repeated JSON commands are byte-identical;
- reviewer text and JSON counts agree;
- raw `compass diff HEAD~1 HEAD` still uses the pre-existing output;
- any `proven_break` has exact identity, exact evidence, complete coverage, and no profile mismatch.

- [ ] **Step 6: Commit documentation and acceptance coverage**

Run:

```bash
git add README.md crates/compass-cli/tests/history_cli.rs
git commit -m "test: verify semantic history replay"
```


---

## Phase 0–1 Definition of Done

- [ ] `compass diff OLD NEW` raw text and JSON behavior remain compatible.
- [ ] `compass diff OLD NEW --semantic` produces concise reviewer text.
- [ ] `compass diff OLD NEW --semantic --format json` emits `compass.semantic_delta.report/1`.
- [ ] Same immutable inputs produce byte-identical JSON and stable 64-character fingerprints.
- [ ] Historical rebuilds do not leak temporary-worktree identity into graph node IDs or edge endpoints.
- [ ] Public entity removal and visibility narrowing are only proven breaks under exact, complete, comparable evidence.
- [ ] Signature changes remain conservative (`possible_break` or `indeterminate`) until language-aware contract proofs exist.
- [ ] Dependency and call changes distinguish exact, inferred, and ambiguous evidence.
- [ ] Profile mismatches are visible as structured limitations and cannot yield proven conclusions.
- [ ] Focused tests, complete crate tests, clippy, formatting, graph update, and the cocoindex smoke test pass.
- [ ] Existing user changes in `compass-graph` remain untouched and uncommitted unless the user separately authorizes them.
