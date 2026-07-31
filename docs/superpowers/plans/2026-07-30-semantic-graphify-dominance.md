# Semantic Graphify Dominance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking. The user explicitly requires
> implementation-first sequencing; production changes precede their focused
> regression tests.

**Goal:** Improve Compass's native graph quality on Django and Entire by fixing
path-sensitive cache identity, retaining empty Python modules and qualified
external calls, recognizing stronger canonical type ownership as semantic
dominance, and closing the measured JavaScript prototype-method gap without
duplicating inferior Graphify placeholders.

**Architecture:** Production extraction and resolution remain independent of
Graphify. Compass emits anchored native facts and bounded unresolved
placeholders; the development-only Python comparator proves exact coverage or
bounded semantic dominance using source, relationship, language, module, and
ownership evidence. Each root-cause correction is implemented first, receives
post-change regression coverage, is verified in its owning crate, and is
committed independently.

**Tech Stack:** Rust 2024, tree-sitter, serde/serde_json, SQLite, Python 3
standard library, Cargo, the existing `benchmarks/performance` harness,
Graphify 0.9.30 as an explicit development oracle.

## Execution status

Tasks 1-7 are implemented and locally verified. Task 8 was executed against
both the pinned Graphify 0.9.30 baseline and a fresh Graphify 0.9.31 checkout.
The phase substantially improved semantic coverage and retained valid,
deterministic graphs, but the deliberately strict final gate remains open:
Django has 348 missing and 25 ambiguous Graphify nodes plus 8,355 missing and
26 ambiguous edges; Entire has 50 missing and 6 ambiguous nodes plus 1,347
missing and 18 ambiguous edges. Current Graphify also improved enough that the
fresh cold-build ratios are below 5x. The review document records complete
measurements, query oracles, and the next evidence-backed gaps.

Before delivery, `origin/main` advanced with project-scoped framework evidence
gating and was merged into this branch. That integration requires SvelteKit,
Nuxt, and Astro file-route fixtures to carry their real package dependencies;
the qualification corpus now has a separate manifest in each subproject so
positive evidence cannot leak into negative fixtures. The exact merged tree
passes all 24 semantic flows and produces byte-identical clean, warm, forced,
restored, and alternate-checkout graphs. A CI-observed transient Windows
`MoveFileExW` denial during generation publication is also handled by bounded,
Windows-only atomic-replacement retries.

## Background and current evidence

The performance branch is `codex/compass-performance-hardening`; draft PR #86
targets `main`. It already contains the correctness-gated benchmark harness,
cache path identity hardening, unchanged-build reuse, query ranking fixes, and
cold graph-publication optimizations.

Pinned real repositories:

- Django `50d706d0aebcc2d073c8d034b6e22fc98fad49f2` (Python plus vendored
  JavaScript).
- Entire `279b988597f1037c14cdd4c46765a5552e067d17` (Go plus scripts).

Qualified baseline:

| Repository | Compass cold p50 | Compass warm p50 | Graphify cold | Cold speedup |
| --- | ---: | ---: | ---: | ---: |
| Django | 12.402s | 1.949s | 66.58s | 5.37x |
| Entire | 4.28s | 0.82s | 25.55s | 5.97x |

Literal cross-schema comparison:

| Repository | Missing Graphify nodes | Missing Graphify edges |
| --- | ---: | ---: |
| Django | 1,175 | 10,600 |
| Entire | 720 | 7,470 |

The literal totals combine genuine omissions with representations where
Compass is stronger:

- In Entire, 668 of 720 missing Graphify nodes have a same-named,
  source-anchored Compass definition.
- 5,159 of 5,176 missing Entire `references` edges already target the
  corresponding canonical Compass definition.
- Graphify repeats Go receiver/type placeholders at generated method sites;
  Compass owns those methods under the declared type.
- Graphify sometimes emits file-to-method containment; Compass emits
  file-to-type-to-method ownership.
- Django contains 610 empty represented Python files that the current pipeline
  deliberately clears after extraction.
- Django loses qualified external member calls such as `mock.patch` when their
  definitions are outside the corpus.
- Django's vendored Select2 bundle has 172 prototype-assigned methods present
  in Graphify but absent from the measured Compass graph.

## Diagnosed root causes

### Cache regression

`Cache::open` canonicalizes its root, while callers can retain a lexical macOS
path through `/var`. The performance branch added the logical path to every
cache key using `path.strip_prefix(self.root)`. A save using
`/private/var/.../a.md` and a load using `/var/.../a.md` therefore produce
different keys. Canonicalizing the leaf is forbidden because it would collapse
logical symlink identities such as `AGENTS.md` and `CLAUDE.md`.

### Empty Python modules

`Engine::extract_source_combined` already creates a file node for an empty
generic Python syntax tree. `compass-core/src/pipeline.rs` then explicitly
clears every graph bucket for any empty non-structured source. The omission is
in pipeline policy, not the Python extractor.

### Generated types and imported bases

Graphify retains use-site/receiver placeholders. Compass has existing
source-scoped and unique-stub rewrites, but Python re-export chains need
qualified import evidence to reach their anchored definitions. Generated Go
receiver placeholders should remain canonicalized, not republished.

### Qualified external calls

Python raw-call extraction records a receiver and call site. Resolution drops a
member call when the imported module is outside the indexed corpus because no
target node exists. The exact occurrence is known; only the endpoint remains
unresolved. A module-qualified, source-scoped external placeholder is therefore
more truthful than omitting the call or joining it to a global common-name hub.

### JavaScript prototype methods

The generic JavaScript declaration walk does not treat assignments such as
`Results.prototype.hideLoading = function () {}` as method declarations. The
function body and call sites are parseable; the missing behavior is a bounded
assignment-expression declaration form.

## Production data flow

```text
selected source file
  -> compass-languages extraction + raw calls/import evidence
  -> compass-core collection and cache
  -> compass-resolve canonicalization and unresolved endpoint retention
  -> compass-graph v1 source-scoped placeholder normalization
  -> graph.json
  -> development-only SQLite exact/dominance comparison
  -> correctness-gated performance eligibility
```

## File structure

- Modify `crates/compass-files/src/cache.rs`: preserve lexical and canonical
  roots and derive one stable relative source key from either spelling.
- Modify `crates/compass-files/tests/contracts.rs`: cover root aliases,
  identical content at distinct logical paths, and leaf symlinks.
- Modify `crates/compass-core/src/pipeline.rs`: retain generic empty-file graph
  facts while preserving explicit structured-document errors.
- Modify `crates/compass-core/tests/code_graph_v1_determinism.rs` or the nearest
  pipeline test module: verify empty Python file nodes and deterministic
  imports.
- Audit and complete `crates/compass-languages/src/engine.rs`: qualified Python
  import metadata and bounded JavaScript prototype methods.
- Audit and complete `crates/compass-model/src/provenance.rs`: the
  `PythonImportedTypeResolution` rewrite rule.
- Audit and complete `crates/compass-resolve/src/lib.rs`: resolve imported
  Python types and retain safely qualified unresolved external calls.
- Modify `crates/compass-resolve/tests/python_import_provenance.rs`: verify
  re-exported bases and unresolved call occurrence evidence.
- Modify `benchmarks/performance/compass/correctness.py`: index semantic
  identity, occurrence, and ownership evidence and classify exact, dominated,
  ambiguous, and missing facts.
- Modify `benchmarks/performance/tests/test_correctness.py`: deterministic
  dominance and fail-closed comparator coverage.
- Update
  `docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md`: final
  quality and performance measurements.

## Global constraints

- Do not add Graphify, Python, or network access to Compass production paths.
- Never accept label equality alone as semantic dominance.
- A dominance candidate set larger than one fails closed as ambiguous.
- Containment dominance is unique, structural, same-file, and at most two
  ownership hops.
- Prefer one anchored definition over any number of use-site placeholders.
- An unresolved external call must have an exact AST occurrence and a safely
  qualified or source-scoped endpoint.
- Empty-file support creates one file node and no invented body symbol.
- Graphs remain deterministic and publish zero validation errors.
- Django and Entire cold build p50 and peak RSS must not regress by more than
  10% from the baseline above.
- Both cold builds must remain at least 5x faster than the measured Graphify
  baseline.
- Preserve unrelated user changes and stage only task-owned paths.
- After code changes, run `graphify update .` in the parent Graphify repository.

---

### Task 1: Reconcile the existing uncommitted language and resolver work

**Files:**

- Audit: `crates/compass-languages/src/engine.rs`
- Audit: `crates/compass-model/src/provenance.rs`
- Audit: `crates/compass-resolve/src/lib.rs`

**Interfaces:**

- Consumes: the approved design and current working-tree diff.
- Produces: a stable, understood starting diff; no code is discarded.

- [ ] **Step 1: Capture and classify the current diff**

Run:

```bash
git status -sb
git diff --stat
git diff -- crates/compass-languages/src/engine.rs \
  crates/compass-model/src/provenance.rs \
  crates/compass-resolve/src/lib.rs
```

Classify each hunk as qualified Python import evidence, imported-type
resolution, JavaScript prototype extraction, focused coverage, or unrelated.
Preserve every unrelated hunk.

- [ ] **Step 2: Format and compile the pre-existing changes**

Run:

```bash
cargo fmt --all
cargo check -p compass-languages -p compass-model -p compass-resolve
```

Expected: compilation succeeds. If it fails, trace the first compiler error to
its originating hunk before making a correction.

- [ ] **Step 3: Run the focused tests already included in the diff**

Run:

```bash
cargo test -p compass-languages javascript_prototype_assignments_are_methods_with_callable_bodies -- --exact
cargo test -p compass-languages python_parenthesized_imports_qualify_inherited_types -- --exact
cargo test -p compass-resolve python_import_reexports_anchor_inheritance_to_the_definition -- --exact
```

Expected: all present tests pass. If a named test is absent after the concurrent
edit stabilizes, record that fact and add equivalent post-change coverage in
the owning task.

- [ ] **Step 4: Commit only verified pre-existing behavior**

If all three files form one coherent verified change:

```bash
git add crates/compass-languages/src/engine.rs \
  crates/compass-model/src/provenance.rs \
  crates/compass-resolve/src/lib.rs
git commit -m "fix(graph): qualify imported types and prototype methods"
```

If the JavaScript and Python changes are independently stageable, split them
into `fix(graph): resolve imported Python types` and
`fix(graph): extract prototype-assigned methods`.

### Task 2: Normalize cache root aliases without resolving leaf identity

**Files:**

- Modify: `crates/compass-files/src/cache.rs`
- Modify: `crates/compass-files/tests/contracts.rs`
- Verify: `crates/compass-semantic/src/tests.rs`

**Interfaces:**

- Consumes: `Cache::open(root, options)` and `source_cache_key`.
- Produces: `Cache { root, logical_root, ... }` and
  `fn logical_source_path(&self, path: &Path) -> Cow<'_, str>`.

- [ ] **Step 1: Implement lexical/canonical root normalization**

Retain a lexical absolute root before canonicalizing:

```rust
let requested_root = root.as_ref();
let logical_root = if requested_root.is_absolute() {
    requested_root.to_path_buf()
} else {
    std::env::current_dir()
        .map_err(|source| io_error(requested_root, source))?
        .join(requested_root)
};
let root =
    fs::canonicalize(requested_root).map_err(|source| io_error(requested_root, source))?;
```

Store `logical_root` on `Cache`. Derive the cache-key path without canonicalizing
the leaf:

```rust
fn logical_source_path<'a>(&'a self, path: &'a Path) -> Cow<'a, str> {
    path.strip_prefix(&self.logical_root)
        .or_else(|_| path.strip_prefix(&self.root))
        .unwrap_or(path)
        .to_string_lossy()
}
```

Use that value in `source_cache_key`.

- [ ] **Step 2: Add post-change root-alias and leaf-symlink coverage**

Add one cache contract that:

1. creates a real root and a directory symlink alias;
2. opens the cache through the alias;
3. saves through the canonical file path;
4. loads through the aliased file path; and
5. proves a symlinked leaf still has a different cache entry from its target.

Use `#[cfg(unix)]` for symlink creation and retain the existing
identical-content logical-path test on all platforms.

- [ ] **Step 3: Verify cache and reproduced semantic-cache failures**

Run:

```bash
cargo fmt --all
cargo test -p compass-files --test contracts
cargo test -p compass-semantic --lib corpus -- --test-threads=1
cargo test -p compass-semantic --lib semantic_cache_checkpoints -- --test-threads=1
```

Expected: the previously reproduced cache-hit assertions pass on arm64 macOS.

- [ ] **Step 4: Commit**

```bash
git add crates/compass-files/src/cache.rs crates/compass-files/tests/contracts.rs
git commit -m "fix(cache): normalize lexical root aliases"
```

### Task 3: Retain deterministic file nodes for empty Python modules

**Files:**

- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/tests/code_graph_v1_determinism.rs`

**Interfaces:**

- Consumes: `Engine::extract_source_combined` output for zero-byte generic code.
- Produces: one exact code-file node for an empty selected Python file.

- [ ] **Step 1: Implement the empty-file policy correction**

Keep the explicit error for empty structured documents. Remove the generic
branch that clears `combined.graph` for all other empty files. Do not synthesize
symbols:

```rust
if empty_structured_document && combined.graph.error.is_none() {
    combined.graph.error = Some(format!("{language} extraction failed: empty document"));
}
```

Allow the existing generic extractor file node to continue through caching,
resolution, and publication.

- [ ] **Step 2: Add post-change pipeline coverage**

Build a fixture with:

```text
pkg/__init__.py       # zero bytes
pkg/__main__.py       # zero bytes
consumer.py           # imports pkg.__main__
```

Assert that the two empty files each publish one file node, the import targets
the `__main__.py` node, and two forced builds have identical graph bytes.

- [ ] **Step 3: Verify core graph behavior**

Run:

```bash
cargo fmt --all
cargo test -p compass-core code_graph_v1
cargo test -p compass-core pipeline
cargo test -p compass-graph
```

- [ ] **Step 4: Commit**

```bash
git add crates/compass-core/src/pipeline.rs \
  crates/compass-core/tests/code_graph_v1_determinism.rs
git commit -m "fix(graph): retain empty Python module nodes"
```

### Task 4: Retain safely qualified unresolved Python calls

**Files:**

- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-resolve/src/lib.rs`
- Modify: `crates/compass-resolve/tests/python_import_provenance.rs`

**Interfaces:**

- Consumes: `RawCall.receiver`, Python import bindings, exact occurrence
  attributes, and existing resolved call edges.
- Produces:
  `fn retain_qualified_python_external_calls(...) -> Vec<EdgeRecord>` plus
  source-scoped placeholder nodes compatible with graph-v1 validation.

- [ ] **Step 1: Implement qualified call metadata and endpoint retention**

For `from unittest import mock`, retain `mock -> unittest.mock`. For a raw
member call `mock.patch`, derive `unittest.mock.patch`. If no unique anchored
target or existing call edge covers the occurrence:

```rust
let placeholder_id = make_id(&[
    "external",
    "python",
    qualified_target,
    &repository_scope(&raw.source_file),
]);
```

Create one sourceless code node with label `patch`, qualified name
`unittest.mock.patch`, language `python`, and explicit unresolved/external
marker attributes. Emit a `calls` edge from `raw.caller_nid` with the raw
call's exact source anchor and extracted syntax evidence.

Do not emit a placeholder when import qualification is ambiguous or when a
unique anchored target exists.

- [ ] **Step 2: Add post-change external-call coverage**

Cover:

- `from unittest import mock; mock.patch(...)`;
- `from other import mock; mock.patch(...)` in another file;
- two files importing the same qualified external symbol;
- an in-repository `mock.patch` definition suppressing the placeholder; and
- an unqualified `patch()` with multiple possible imports failing closed.

Assert exact occurrence anchors and prove that unrelated modules never merge
into one placeholder hub.

- [ ] **Step 3: Verify resolution and graph-v1 placeholder contracts**

Run:

```bash
cargo fmt --all
cargo test -p compass-languages python
cargo test -p compass-resolve python
cargo test -p compass-resolve --test python_import_provenance
cargo test -p compass-graph sourceless_placeholder
cargo test -p compass-graph unresolved_external
```

- [ ] **Step 4: Commit**

```bash
git add crates/compass-languages/src/engine.rs \
  crates/compass-resolve/src/lib.rs \
  crates/compass-resolve/tests/python_import_provenance.rs
git commit -m "fix(graph): retain qualified external Python calls"
```

### Task 5: Classify exact and semantically dominated Graphify facts

**Files:**

- Modify: `benchmarks/performance/compass/correctness.py`
- Modify: `benchmarks/performance/tests/test_correctness.py`
- Modify: `benchmarks/performance/tests/fixtures/compass_graph.json`
- Modify: `benchmarks/performance/tests/fixtures/graphify_graph.json`

**Interfaces:**

- Consumes: streamed Compass v1 and Graphify node-link graphs.
- Produces: exact, dominated, ambiguous, and missing node/edge metrics with
  reason codes.

- [ ] **Step 1: Extend the temporary semantic index**

Add node columns for normalized label, qualified name, language family,
placeholder status, and anchored-definition status. Add edge columns for
context, occurrence file/location, and normalized source/target identity.

Derive language conservatively from explicit metadata, then source extension.
Derive placeholders only from sourceless/use-site evidence; never from a label.
Recreate the task-owned SQLite database for each comparison, so no migration is
needed.

- [ ] **Step 2: Implement bounded dominance classification**

Add pure classification helpers:

```python
@dataclass(frozen=True)
class Coverage:
    status: str  # "exact", "dominated", "ambiguous", "missing"
    reason: str
    compass_fact: str | None
```

Implement:

- `resolved_endpoint`: same relation/source occurrence, compatible language and
  module, Graphify placeholder/use-site target, unique anchored Compass target;
- `canonical_owner`: generated receiver/type occurrence uniquely owned by the
  declared Compass type;
- `containment_path`: one unique same-file structural path of at most two hops.

Return `ambiguous`, not `dominated`, when more than one candidate survives.

- [ ] **Step 3: Update comparison metrics and failures**

Report:

```text
exact_graphify_nodes
dominated_graphify_nodes
ambiguous_graphify_nodes
missing_graphify_nodes
exact_graphify_edges
dominated_graphify_edges
ambiguous_graphify_edges
missing_graphify_edges
```

Include reason-grouped examples. `CorrectnessResult.passed` is false only for
in-scope ambiguous or missing facts, validation errors, or graph invariants.
The full report must still show deferred JavaScript gaps until Task 6 qualifies
them.

- [ ] **Step 4: Add post-change fail-closed tests**

Fixtures must prove:

- a sourceless Graphify Go receiver target is dominated by one anchored Compass
  type;
- same-label cross-package definitions are ambiguous;
- a file-to-type-to-method path dominates flat file-to-method containment;
- a three-hop path does not dominate;
- different relationship sites do not dominate;
- genuine missing nodes and edges still fail; and
- output and digest are deterministic across repeated comparisons.

- [ ] **Step 5: Verify the performance harness**

Run:

```bash
python3 -m unittest discover -s benchmarks/performance/tests -v
python3 -m compileall -q benchmarks/performance
```

- [ ] **Step 6: Commit**

```bash
git add benchmarks/performance/compass/correctness.py \
  benchmarks/performance/tests/test_correctness.py \
  benchmarks/performance/tests/fixtures/compass_graph.json \
  benchmarks/performance/tests/fixtures/graphify_graph.json
git commit -m "feat(perf): compare semantically dominant graph facts"
```

### Task 6: Qualify JavaScript prototype-assigned methods

**Files:**

- Modify if required: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/tests/semantic_producers.rs`

**Interfaces:**

- Consumes: JavaScript/TypeScript `assignment_expression` with a function-valued
  right side.
- Produces: a method node, structural owner edge, and callable body for
  `Owner.prototype.name` and bounded jQuery-style `Owner.fn.name`.

- [ ] **Step 1: Complete the bounded production implementation**

Accept only:

```text
Owner.prototype.method = function (...) { ... }
Owner.prototype.method = (...) => ...
Owner.fn.method = function (...) { ... }
```

Reject arbitrary object-property assignments. Prefer an extracted constructor
or type owner; otherwise contain the method under the file while preserving
`lexical_owner` and qualified name.

- [ ] **Step 2: Add post-change producer coverage**

Verify prototype methods, `.fn` methods, calls inside their bodies, duplicate
method names under different owners, and rejection of ordinary
`config.render = function` assignments.

- [ ] **Step 3: Verify language extraction**

Run:

```bash
cargo fmt --all
cargo test -p compass-languages javascript
cargo test -p compass-languages typescript
cargo test -p compass-languages --test semantic_producers
```

- [ ] **Step 4: Commit**

```bash
git add crates/compass-languages/src/engine.rs \
  crates/compass-languages/tests/semantic_producers.rs
git commit -m "fix(graph): extract prototype-assigned methods"
```

Skip this commit if Task 1 already committed the exact verified behavior and no
additional changes are required.

### Task 7: Run full local verification

**Files:** no intended source changes.

- [ ] **Step 1: Verify formatting and relevant crates**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-files --test contracts
cargo test -p compass-semantic --lib
cargo test -p compass-languages
cargo test -p compass-resolve
cargo test -p compass-graph
cargo test -p compass-core code_graph_v1
python3 -m unittest discover -s benchmarks/performance/tests -v
git diff --check
```

- [ ] **Step 2: Verify release build**

Run:

```bash
cargo build --release -p compass-cli
```

Record the binary commit and SHA-256 in the final qualification report.

### Task 8: Rebuild and compare Django and Entire

**Files:**

- Update:
  `docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md`
- Produce ignored artifacts under `target/performance/`.

- [ ] **Step 1: Rebuild each pinned corpus from a clean owned output**

Use the existing harness with the exact repository pins in
`benchmarks/performance/repositories.toml`. Run at least three cold, warm, and
incremental Compass samples. Run one explicit fresh Graphify 0.9.30 comparison
per corpus and reuse no Graphify output across repository revisions.

- [ ] **Step 2: Inspect semantic coverage**

For Django and Entire, record exact, dominated, ambiguous, and missing counts by
relation and reason. Manually inspect at least:

- one empty Python `__init__.py`;
- one `mock.patch` call;
- one re-exported Django base class;
- one canonical generated Go receiver;
- one two-hop containment dominance case; and
- five Select2 prototype methods across different owners.

- [ ] **Step 3: Enforce performance and quality gates**

Reject the result if:

- either graph has validation errors;
- repeated Compass graph digests differ;
- a dominance match lacks a unique proof;
- Django cold p50 exceeds 13.642s or warm p50 exceeds 2.144s;
- Entire cold p50 exceeds 4.708s or warm p50 exceeds 0.902s;
- peak RSS exceeds the existing Compass baseline by more than 10%; or
- either cold Graphify speedup falls below 5x.

- [ ] **Step 4: Update the tracked review**

Document commands, commits, machine, tool versions, counts, timings, memory,
remaining genuine gaps, and representative exact/dominance evidence. Do not
claim literal Graphify equality when the result is semantic dominance.

- [ ] **Step 5: Commit the verified baseline**

```bash
git add docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md
git commit -m "docs: record semantic graph quality qualification"
```

### Task 9: Refresh Graphify knowledge and publish

**Files:**

- Parent repository generated graph under `/Users/haipingfu/graphify/graphify-out/`.

- [ ] **Step 1: Refresh the parent knowledge graph**

Run:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

Record extractor dependency warnings without hiding them.

- [ ] **Step 2: Perform final branch verification**

Run:

```bash
cd /Users/haipingfu/graphify/compass/.worktrees/compass-performance-hardening
git status -sb
git diff --check
git log --oneline origin/main..HEAD
git rev-list --left-right --count HEAD...origin/codex/compass-performance-hardening
```

Expected: intended changes are committed, no task-owned modifications remain,
and the branch is ready to push.

- [ ] **Step 3: Push and update PR #86**

```bash
git push -u origin codex/compass-performance-hardening
gh pr view 86 --json url,title,isDraft,headRefOid,statusCheckRollup
```

Update the PR body with final semantic-quality counts, performance results,
remaining risks, and exact verification commands. Keep the PR draft until CI is
green or only a failure reproduced on `main` remains.
