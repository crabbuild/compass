# Extraction Superset Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Compass a correctness-preserving superset of Graphify on shared Podman inputs while keeping release medians at or below 32 seconds cold and 10 seconds warm.

**Architecture:** Correct C-family names at the syntax-tree boundary, prevent entity deduplication from merging positional document facts, and version the AST cache when extraction semantics change. Add development-only graph comparison and qualification tools so Podman parity and timing remain reproducible without adding Graphify or Python to the Compass runtime.

**Tech Stack:** Rust 2024, tree-sitter, serde_json, Cargo tests, POSIX shell, Compass CLI, Graphify Python CLI

## Global Constraints

- Graphify shared nodes must be a subset of Compass nodes.
- Graphify shared edges must be a subset of Compass edges.
- Valid Compass-only Perl and extensionless-script facts must remain present.
- Shared node IDs, labels, types, source paths, and available locations must agree.
- Derived community assignments are outside the parity contract.
- Podman release medians must remain at or below 32 seconds cold and 10 seconds warm across three runs each.
- Correctness takes priority over timing.
- Production Compass must not depend on Graphify, Python, or the network.
- The user explicitly waived test-first development. Add and run focused regression tests immediately after each production change.
- Preserve the unrelated working-tree change in `crates/compass-output/src/html.rs` and the untracked `advisor-plans/`, `codegraph/`, and `graphify-out/` paths.

---

## File structure

- Modify `crates/compass-languages/src/engine.rs`: resolve C and C++ function names from declarators before generic declaration fallback.
- Modify `crates/compass-parity/src/lib.rs`: add exact Graphify oracle coverage for macro-heavy C and nested C++ declarators.
- Modify `crates/compass-graph/src/dedup.rs`: classify positional nodes and exclude them from label-based entity merging.
- Modify `crates/compass-files/src/cache.rs`: advance and name the AST extraction cache version.
- Modify `crates/compass-files/tests/contracts.rs`: verify the new namespace and stale-cache cleanup.
- Modify `crates/compass-parity/Cargo.toml`: expose serde_json to the development-only comparison binary.
- Create `crates/compass-parity/src/bin/compare_graphs.rs`: compare Graphify nodes and edges as required subsets and report Compass-only facts.
- Create `scripts/qualify_graphify_superset.sh`: reset explicit output directories, build release Compass, run cold/warm/query samples, and invoke the comparator.
- Modify `docs/implementation/extraction-pipeline.md`: document the compatibility gate, cache rebuild, and qualification command.
- Create `CHANGELOG.md`: establish an Unreleased section that describes corrected C-family names, preserved positional nodes, and the one-time AST cache refresh.

### Task 1: Resolve C and C++ callable declarators

**Files:**

- Modify: `crates/compass-languages/src/engine.rs:1227`
- Modify: `crates/compass-parity/src/lib.rs:2313`

**Interfaces:**

- Consumes: tree-sitter `Node`, `ExtractState::node_text`, and the current `function_name` dispatch.
- Produces: `ExtractState::c_family_function_name(Node) -> Option<String>` and `c_family_declarator_name(Node) -> Option<Node>`.

- [ ] **Step 1: Add declarator-first production resolution**

Add a C-family branch before the generic declaration-name path:

```rust
fn function_name(&self, node: Node<'tree>) -> Option<String> {
    if matches!(self.language, "c" | "cpp") {
        return self
            .c_family_function_name(node)
            .or_else(|| self.declaration_name(node));
    }
    self.declaration_name(node).or_else(|| {
        node.child_by_field_name("declarator")
            .and_then(first_identifier)
            .and_then(|name| self.node_text(name))
            .map(clean_name)
    })
}

fn c_family_function_name(&self, node: Node<'tree>) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = c_family_declarator_name(declarator)?;
    self.node_text(name)
        .map(clean_name)
        .filter(|name| !name.is_empty())
}
```

Add a bounded resolver beside the identifier helpers. It follows named declarator fields first and accepts callable terminal forms without scanning the declaration prefix:

```rust
fn c_family_declarator_name(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(
        node.kind(),
        "identifier"
            | "field_identifier"
            | "qualified_identifier"
            | "destructor_name"
            | "operator_name"
    ) {
        return Some(node);
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return c_family_declarator_name(inner);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(name) = c_family_declarator_name(child) {
            return Some(name);
        }
    }
    None
}
```

Keep the traversal inside the function's declarator subtree. Do not call `first_identifier` on the full declaration.

- [ ] **Step 2: Format and compile the language crate**

Run:

```bash
cargo fmt --all
cargo check -p compass-languages
```

Expected: both commands exit successfully.

- [ ] **Step 3: Add post-change regression coverage**

Add temporary-file parity cases to `crates/compass-parity/src/lib.rs`:

```rust
#[test]
fn macro_heavy_c_declarators_match_graphify() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("declarators.c");
    fs::write(
        &source,
        "SQLITE_PRIVATE char *sqlite3CompileOptions(void) { return 0; }\n\
         static struct Stat *sqlite3StatType(int value) { return 0; }\n\
         SQLITE_API sqlite3_int64 sqlite3StatusValue(void) { return 0; }\n",
    )?;
    compare_extraction_path(&source, "extract_c")?;
    Ok(())
}

#[test]
fn nested_cpp_declarators_match_graphify() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("declarators.cpp");
    fs::write(
        &source,
        "int DBImpl::Compact() { return 1; }\n\
         DBImpl::~DBImpl() {}\n\
         bool DBImpl::operator==(const DBImpl&) const { return true; }\n",
    )?;
    compare_extraction_path(&source, "extract_cpp")?;
    Ok(())
}
```

Also add a direct engine test that asserts the three C labels are `sqlite3CompileOptions()`, `sqlite3StatType()`, and `sqlite3StatusValue()`, and that `char()`, `struct()`, and `sqlite3_int64()` are absent.

- [ ] **Step 4: Run focused and existing extraction parity tests**

Run:

```bash
cargo test -p compass-languages
cargo test -p compass-parity tests::macro_heavy_c_declarators_match_graphify -- --exact
cargo test -p compass-parity tests::nested_cpp_declarators_match_graphify -- --exact
cargo test -p compass-parity tests::c_ast_extraction_matches_exactly -- --exact
cargo test -p compass-parity tests::cpp_ast_extraction_matches_exactly -- --exact
```

Expected: every command exits successfully and exact extraction matches the Graphify oracle.

- [ ] **Step 5: Commit the callable-name correction**

```bash
git add crates/compass-languages/src/engine.rs crates/compass-parity/src/lib.rs
git commit -m "fix: resolve C family callable declarators"
```

### Task 2: Preserve positional document and rationale nodes

**Files:**

- Modify: `crates/compass-graph/src/dedup.rs:84`

**Interfaces:**

- Consumes: `NodeRecord.attributes["file_type"]`.
- Produces: `is_entity_merge_candidate(&NodeRecord) -> bool`, used by exact and fuzzy label merging.

- [ ] **Step 1: Exclude positional facts from entity merging**

Add:

```rust
fn is_positional(node: &NodeRecord) -> bool {
    matches!(
        string_attribute(node, "file_type").as_deref(),
        Some("document" | "rationale")
    )
}

fn is_entity_merge_candidate(node: &NodeRecord) -> bool {
    !is_code(node) && !is_positional(node)
}
```

Replace both `if is_code(node) { continue; }` guards in exact and fuzzy candidate construction with:

```rust
if !is_entity_merge_candidate(node) {
    continue;
}
```

Keep `collapse_id_collisions` unchanged so literal duplicate records with the same canonical ID still collapse.

- [ ] **Step 2: Add post-change positional identity tests**

Add a helper that assigns `file_type`, `source_location`, and a line-qualified ID. Add these cases:

```rust
#[test]
fn repeated_positional_nodes_in_one_file_remain_distinct() -> Result<(), DedupError> {
    let nodes = vec![
        positional_node("release_notes_heading_l10", "Fixed", "RELEASE_NOTES.md", "L10", "document"),
        positional_node("release_notes_heading_l40", "Fixed", "RELEASE_NOTES.md", "L40", "document"),
        positional_node("decision_l12", "Preserve compatibility", "ADR.md", "L12", "rationale"),
        positional_node("decision_l60", "Preserve compatibility", "ADR.md", "L60", "rationale"),
    ];
    let result = deduplicate_entities(&nodes, &[], &HashMap::new())?;
    assert_eq!(result.nodes.len(), 4);
    assert_eq!(result.stats.removed, 0);
    Ok(())
}

#[test]
fn literal_positional_id_collisions_still_collapse() -> Result<(), DedupError> {
    let node = positional_node(
        "release_notes_heading_l10",
        "Fixed",
        "RELEASE_NOTES.md",
        "L10",
        "document",
    );
    let result = deduplicate_entities(&[node.clone(), node], &[], &HashMap::new())?;
    assert_eq!(result.nodes.len(), 1);
    Ok(())
}
```

- [ ] **Step 3: Run focused and graph tests**

Run:

```bash
cargo fmt --all
cargo test -p compass-graph dedup
cargo test -p compass-graph
cargo test -p compass-parity tests::deterministic_entity_dedup_matches_python -- --exact
```

Expected: positional repeats survive, ID collisions collapse, existing concept dedup remains unchanged, and the existing cross-file Graphify oracle case still passes.

- [ ] **Step 4: Commit positional identity preservation**

```bash
git add crates/compass-graph/src/dedup.rs
git commit -m "fix: preserve positional document identities"
```

### Task 3: Invalidate stale AST extraction entries once

**Files:**

- Modify: `crates/compass-files/src/cache.rs:62`
- Modify: `crates/compass-files/tests/contracts.rs:480`

**Interfaces:**

- Produces: `const AST_EXTRACTOR_VERSION: &str = "0.9.21"` as the default AST namespace.
- Preserves: `Cache::with_extractor_version` for isolated compatibility tests.

- [ ] **Step 1: Name and advance the default AST version**

Add the named constant and use it in `Cache::new`:

```rust
const AST_EXTRACTOR_VERSION: &str = "0.9.21";

let cache = Self {
    root,
    cache_root,
    output_name,
    extractor_version: AST_EXTRACTOR_VERSION.to_owned(),
    hashes,
    session_hashes: HashMap::new(),
};
```

This changes only the AST directory from `cache/ast/v0.9.20` to `cache/ast/v0.9.21`. Existing semantic caches remain eligible.

- [ ] **Step 2: Extend the cache contract test**

Before the custom-version assertions, instantiate a default cache in an isolated cache root and assert:

```rust
let default_cache = Cache::new(&root, Some(&cache_root))?;
assert!(
    default_cache
        .directory(&CacheKind::Ast, None)
        .ends_with("ast/v0.9.21")
);
assert!(!cache_root.join("compass-out/cache/ast/v0.9.20").exists());
```

Create `v0.9.20/stale.json` in the fixture setup so the cleanup assertion proves a semantic version change invalidates the old AST namespace.

- [ ] **Step 3: Run cache and incremental pipeline verification**

Run:

```bash
cargo fmt --all
cargo test -p compass-files cache_versions_legacy_fingerprints_pruning_and_cleanup_are_total -- --exact
cargo test -p compass-files
cargo test -p compass-core update
```

Expected: the old AST namespace is removed, the new namespace is selected, and incremental update tests pass.

- [ ] **Step 4: Commit cache invalidation**

```bash
git add crates/compass-files/src/cache.rs crates/compass-files/tests/contracts.rs
git commit -m "fix: invalidate stale extraction cache"
```

### Task 4: Add a deterministic superset comparator

**Files:**

- Modify: `crates/compass-parity/Cargo.toml`
- Create: `crates/compass-parity/src/bin/compare_graphs.rs`

**Interfaces:**

- CLI: `cargo run -p compass-parity --bin compare-graphs -- <compass-graph.json> <graphify-graph.json>`
- Produces exit code `0` only when every Graphify node, observable node field, and edge key exists in Compass.
- Edge key: `(source, target, relation)`.
- Observable node fields: `label`, `file_type`, `source_file`, and non-empty `source_location`.

- [ ] **Step 1: Add the development-only binary dependency**

Move `serde_json.workspace = true` into regular dependencies while keeping runtime Compass crates unchanged:

```toml
[dependencies]
serde_json.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Implement graph normalization and comparison**

Create a binary with these concrete records:

```rust
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EdgeKey {
    source: String,
    target: String,
    relation: String,
}

#[derive(Debug, Default)]
struct Report {
    missing_nodes: Vec<String>,
    mismatched_nodes: Vec<String>,
    missing_edges: Vec<EdgeKey>,
    compass_only_nodes: Vec<String>,
    compass_only_edges: Vec<EdgeKey>,
}

impl Report {
    fn compatible(&self) -> bool {
        self.missing_nodes.is_empty()
            && self.mismatched_nodes.is_empty()
            && self.missing_edges.is_empty()
    }
}
```

Load `nodes` by canonical `id`. Load edges from `links`, falling back to `edges`. Compare every Graphify node against the Compass map. For each shared ID, compare `label`, `file_type`, and `source_file`; compare `source_location` only when Graphify provides a non-empty value. Normalize edges into `BTreeSet<EdgeKey>` and compute `graphify_edges.difference(&compass_edges)`.

Print totals and at most 50 representative failures grouped under `missing nodes`, `field mismatches`, and `missing edges`. Print Compass-only counts as informational. Return exit code 1 for an incompatible report and 2 for invalid arguments or unreadable JSON.

- [ ] **Step 3: Add post-change comparator tests**

Use inline JSON fixtures to verify:

```rust
#[test]
fn graphify_subset_with_compass_extras_passes() {
    let graphify = json!({
        "nodes": [{"id":"a","label":"a()","file_type":"code","source_file":"a.c","source_location":"L1"}],
        "links": []
    });
    let compass = json!({
        "nodes": [
            {"id":"a","label":"a()","file_type":"code","source_file":"a.c","source_location":"L1"},
            {"id":"perl_extra","label":"run()","file_type":"code","source_file":"tool"}
        ],
        "links": []
    });
    assert!(compare(&compass, &graphify).unwrap().compatible());
}

#[test]
fn missing_node_field_and_edge_fail() {
    let graphify = json!({
        "nodes": [
            {"id":"a","label":"a()","file_type":"code","source_file":"a.c","source_location":"L1"},
            {"id":"b","label":"b()","file_type":"code","source_file":"a.c","source_location":"L2"}
        ],
        "links": [{"source":"a","target":"b","relation":"calls"}]
    });
    let compass = json!({
        "nodes": [{"id":"a","label":"wrong()","file_type":"code","source_file":"a.c","source_location":"L1"}],
        "links": []
    });
    let report = compare(&compass, &graphify).unwrap();
    assert_eq!(report.missing_nodes, ["b"]);
    assert_eq!(report.mismatched_nodes.len(), 1);
    assert_eq!(report.missing_edges.len(), 1);
}
```

- [ ] **Step 4: Run comparator verification**

Run:

```bash
cargo fmt --all
cargo test -p compass-parity --bin compare-graphs
cargo clippy -p compass-parity --bin compare-graphs -- -D warnings
```

Expected: tests pass and Clippy reports no warnings.

- [ ] **Step 5: Commit the comparator**

```bash
git add crates/compass-parity/Cargo.toml crates/compass-parity/src/bin/compare_graphs.rs Cargo.lock
git commit -m "feat: add graph superset parity checker"
```

### Task 5: Add reproducible Podman qualification and documentation

**Files:**

- Create: `scripts/qualify_graphify_superset.sh`
- Modify: `docs/implementation/extraction-pipeline.md:260`
- Create: `CHANGELOG.md`

**Interfaces:**

- Environment:
  - `PODMAN_ROOT`, default `/Volumes/Workspace/Github/podman`
  - `COMPASS_BIN`, default `target/release/compass` resolved from the Compass repository root
  - `GRAPHIFY_PYTHON`, default `/Users/haipingfu/graphify/.venv/bin/python`
  - `PARITY_SAMPLES`, default `3`
  - `QUERY_SAMPLES`, default `5`
- Outputs: timing TSV and summary under a new temporary directory printed by the script.

- [ ] **Step 1: Implement guarded output reset and release build**

The script must use `set -euo pipefail`, resolve absolute paths, and reject a Podman root that is empty, `/`, the home directory, or lacks `.git`. Set explicit output paths:

```bash
compass_output="$PODMAN_ROOT/compass-out"
graphify_output="$PODMAN_ROOT/graphify-out"
```

Before each destructive reset, verify the target equals one of those two exact paths and its parent equals the validated Podman root. Remove only the validated output directory.

Build:

```bash
cargo build --release -p compass-cli
```

- [ ] **Step 2: Record three cold and warm samples for both tools**

Use `/usr/bin/time -p` around:

```bash
(cd "$PODMAN_ROOT" && COMPASS_OUT=compass-out "$COMPASS_BIN" update .)
(cd "$PODMAN_ROOT" && "$GRAPHIFY_PYTHON" -m graphify update .)
```

For cold samples, reset only the corresponding output directory first. For warm samples, preserve a completed output and run an unchanged update. Record tool, operation, sample, seconds, commit IDs, node count, and edge count as TSV. Sort each three-sample series numerically and select the middle value as the median.

Exit unsuccessfully when Compass cold median exceeds 32 seconds or Compass warm median exceeds 10 seconds.

- [ ] **Step 3: Run parity and query qualification**

After the final complete outputs, invoke:

```bash
cargo run --release -p compass-parity --bin compare-graphs -- \
  "$compass_output/graph.json" \
  "$graphify_output/graph.json"
```

Run each query five times and record medians:

```bash
"$COMPASS_BIN" query "update" --graph "$compass_output/graph.json"
"$GRAPHIFY_PYTHON" -m graphify query "update" --graph "$graphify_output/graph.json"
```

Capture query output outside the timed result. Before reporting latency, normalize the `NODE` and `EDGE` result rows and fail if the Graphify rows are not a subset of the Compass rows.

- [ ] **Step 4: Document the gate and cache change**

Add a `Graphify superset qualification` subsection to `docs/implementation/extraction-pipeline.md` with:

```bash
PODMAN_ROOT=/Volumes/Workspace/Github/podman \
  bash scripts/qualify_graphify_superset.sh
```

Explain that the command deletes only `$PODMAN_ROOT/compass-out` and `$PODMAN_ROOT/graphify-out`, runs both tools from clean outputs, checks shared node and edge inclusion, and records medians. State that Compass-only language facts are informational.

Add a changelog entry explaining that corrected C/C++ declarator semantics use AST cache namespace `v0.9.21`, causing one extraction refresh followed by normal warm reuse.

- [ ] **Step 5: Run local script validation**

Run:

```bash
bash -n scripts/qualify_graphify_superset.sh
git diff --check
```

Expected: shell syntax and whitespace checks pass.

- [ ] **Step 6: Commit qualification tooling and documentation**

```bash
git add scripts/qualify_graphify_superset.sh docs/implementation/extraction-pipeline.md CHANGELOG.md
git commit -m "docs: add Graphify superset qualification"
```

### Task 6: Complete repository and Podman verification

**Files:**

- Modify only if failures reveal a scoped correctness defect in the files named by Tasks 1 through 5.
- Refresh generated graph output with `graphify update .` after all code changes.

**Interfaces:**

- Consumes: the release Compass binary, Graphify environment, Podman checkout, parity comparator, and qualification script.
- Produces: passing workspace checks, zero Graphify-missing nodes and edges, and benchmark evidence.

- [ ] **Step 1: Run repository checks**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

Expected: every command exits successfully.

- [ ] **Step 2: Refresh the required repository graph**

Run from `/Users/haipingfu/graphify`:

```bash
graphify update .
```

Expected: the graph update exits successfully. Do not stage generated `graphify-out/` files unless they are already tracked and intentionally changed by repository policy.

- [ ] **Step 3: Run full Podman qualification**

Run:

```bash
PODMAN_ROOT=/Volumes/Workspace/Github/podman \
  COMPASS_BIN=/Users/haipingfu/graphify/compass/target/release/compass \
  GRAPHIFY_PYTHON=/Users/haipingfu/graphify/.venv/bin/python \
  bash scripts/qualify_graphify_superset.sh
```

Expected:

- Missing Graphify nodes: 0
- Mismatched shared nodes: 0
- Missing Graphify edges: 0
- Compass cold median: at most 32 seconds
- Compass warm median: at most 10 seconds
- Query result inclusion: pass
- Compass-only Perl and extensionless-script facts: reported and retained

- [ ] **Step 4: Inspect the final change set**

Run:

```bash
git status --short
git log --oneline --decorate origin/main..HEAD
git diff --stat origin/main...HEAD
```

Confirm that `crates/compass-output/src/html.rs`, `advisor-plans/`, `codegraph/`, and generated output directories were not included in any task commit.

- [ ] **Step 5: Commit any verification-driven scoped correction**

If Step 3 exposes a defect, return to the task that owns the affected interface, apply its verification commands, and stage only the exact files listed by that task. Use `git commit -m "fix: close remaining graph parity gap"` after the scoped verification passes. If Step 3 passes without a correction, do not create an empty commit.

## Completion evidence

Record in the final handoff:

- Branch and commit list
- Focused and workspace verification commands
- Compass and Graphify node and edge totals
- Missing shared node, field, and edge counts
- All cold, warm, and query samples plus medians
- Compass-only facts retained
- Any unrelated working-tree changes left untouched
