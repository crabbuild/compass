# Graphify origin/main → Compass deep audit

## Executive result

Compass already surpasses the fetched Graphify `origin/main` snapshot in the
areas that define its architecture: native typed Modules, deterministic graph
construction, bounded CompassQL, immutable versioned realizations, Program IR,
resource limits, and evidence-preserving history.

The fetched branch still revealed one user-visible correctness gap and four
high-leverage architectural gaps:

1. Compass wiki exports omit incoming relationships for directed graphs.
2. Compatibility identity is prose spread across files while CI checks out a
   mutable, different upstream line.
3. Query and MCP text drops numeric confidence and relationship provenance.
4. Current-tree artifacts are individually atomic but not published as one
   observable generation.
5. Pull-request and release workflows do not enforce all-target and
   production-path qualification promised by the documentation.

## Snapshot and lineage

| Repository/ref | Commit | Date | Identity |
|---|---|---|---|
| Compass `main` | `3837b411197771351b387cff935e4ae1e0eb8750` | 2026-07-23 | Native Rust workspace |
| Graphify `origin/main` | `91f4d120b630ee35c79bf3c75ccd186870a808f9` | 2026-05-14 | v1 line; package `0.1.14` |
| Compass frozen Graphify oracle | `edec9eabeceeae6aa2375eddb3835efa1a32c0a3` | frozen ledger | Graphify `v0.9.20` on v8 lineage |

`origin/main` and the frozen v8 oracle diverge after
`81a43f028ff1d3fd9a0893318272348a38dad660`. At audit time, Git reported 36
commits unique to main and 1,235 commits unique to `origin/v8`. Therefore
“latest origin/main” means the tip of a legacy-divergent product line, not the
latest behavior on Compass's current oracle lineage.

The audit worktree is:

```text
/private/tmp/graphify-origin-main-audit.kd6DCD/worktree
```

The user's dirty Graphify `v8` checkout and R-support changes were not merged,
rebased, stashed, or modified.

## Main-only capability disposition

| Graphify main capability | Compass status | Evidence summary |
|---|---|---|
| Scored `INFERRED` edges | Present, but not fully surfaced | Numeric scores survive graph/output paths; surprise/query text drops part of the evidence |
| Semantic-similarity edges | Present | Native analysis recognizes `semantically_similar_to` |
| Hyperedges | Surpassed in storage/history; query gap remains | Preserved, reported, shaded in HTML, and versioned |
| Wiki export | Present with a directed-graph correctness gap | Outgoing evidence is rendered; incoming evidence is omitted |
| Code-only watch rebuild | Surpassed | Native watcher also tracks program artifacts and SCIP companions |
| Git hook/worktree fixes | Surpassed in implementation depth | Managed hooks, custom hook paths, background refresh, merge integration, and history |
| Rationale-node prohibition | Defensive cleanup exists; prompt contract drifts | Cleanup converts/removes pseudo-nodes, but embedded prompt still advertises `rationale|concept` |
| Worked mixed-corpus examples | Product/evidence gap | Compass has benchmark machinery but not an equivalent public frozen mixed-corpus walkthrough |
| Leiden clustering | Not parity | Compass ships deterministic Louvain only |

## Verification performed

### Graphify origin/main

Command:

```bash
PYTHONDONTWRITEBYTECODE=1 \
  /Users/haipingfu/graphify/.venv/bin/python \
  -m pytest -q -p no:cacheprovider
```

Result: 253 passed and 40 failed. Every displayed failure was blocked by the
audit environment lacking `graspologic`; the dependency is mandatory on this
Graphify line. No install was performed because the audit was source-read-only.

### Compass against the exact main snapshot

Command:

```bash
GRAPHIFY_REPO_ROOT=/private/tmp/graphify-origin-main-audit.kd6DCD/worktree \
GRAPHIFY_PYTHON=/Users/haipingfu/graphify/.venv/bin/python \
PYTHONDONTWRITEBYTECODE=1 \
cargo test -p compass-parity --locked -- --test-threads=1
```

Result: 24 passed and 74 failed. This is evidence that the current differential
suite is bound to the v8 lineage, not evidence of 74 Compass regressions.
Failures included v8-only commands, modules, fixtures, and extraction contracts
that do not exist on main, plus real behavior differences such as legacy node
IDs and call confidence.

### Focused Compass feature checks

The following passed:

- `cargo test -p compass-output --locked`: 19 tests.
- `cargo test -p compass-core watch_ --locked`: 2 watcher tests.
- `cargo test -p compass-cli --test hook_cli --locked`: 7 hook tests.
- `cargo test -p compass-semantic cleanup_attaches_only_explicit_rationale_and_repairs_hyperedges --locked`: 1 semantic cleanup test.

No source files were changed by these checks.

## Vetted findings

| # | Finding | Category | Impact | Effort | Fix risk | Confidence |
|---|---|---|---|---|---|---|
| 1 | Directed wiki omits incoming caller/dependent evidence | Correctness | HIGH | S | LOW | HIGH |
| 2 | Compatibility lineage and evidence can drift | Dependencies/docs | HIGH | M | MED | HIGH |
| 3 | Path/discovery/MCP drop confidence and provenance | Direction/architecture | HIGH | M | MED | HIGH |
| 4 | Current outputs are not one observable generation | Correctness/architecture | HIGH | L | HIGH | HIGH |
| 5 | PR/release gates do not match qualification claims | Tests/performance | HIGH | M | LOW | HIGH |
| 6 | Natural-query traversal is directionally incomplete in MCP | Correctness | HIGH | M | MED | HIGH |
| 7 | Ghost merging uses basename plus label | Correctness | HIGH | M | MED | HIGH |
| 8 | Edge assembly can collapse distinct relations/evidence sites | Correctness | HIGH | L | HIGH | HIGH |
| 9 | Changed-file updates still redo corpus-sized downstream work | Performance | MED/HIGH at scale | L | HIGH | HIGH |
| 10 | Natural-query scoring scans every node twice | Performance | MED/HIGH at scale | M | MED | HIGH |
| 11 | MCP multi-project graph cache is unbounded | Performance | MED | M | LOW | HIGH |
| 12 | Linux/Windows are tested but not packaged | Distribution | HIGH for adoption | L | MED | HIGH |

### Evidence highlights

- Graphify main uses an undirected `nx.Graph` at
  `graphify/build.py:14-27`; its wiki sees both endpoint directions.
- Compass's `WikiGraph` only records target-side incidence when
  `document.directed` is false at `crates/compass-output/src/wiki.rs:146-189`.
- `COMPATIBILITY.md:7-16` names an immutable v0.9.20 oracle, while
  `.github/workflows/compass-ci.yml:27-41` and
  `.github/workflows/compass-hardening.yml:260-272` check out mutable `v8`.
- `crates/compass-graph/src/analyze.rs:87-98` does not carry numeric confidence
  or relationship source evidence into `SurpriseConnection`.
- `crates/compass-core/src/pipeline.rs:836-960` replaces current artifacts
  sequentially. `BuildGuard::ensure_complete` exists at
  `crates/compass-files/src/build_guard.rs:19-25`, but native readers do not
  consult it.
- PR CI limits workspace tests to `--lib --bins` at
  `.github/workflows/compass-ci.yml:71-91`; all-target execution is scheduled
  hardening, and `.github/workflows/compass-release.yml:28-95` does not depend
  on qualification for the tagged commit.

## Architecture assessment

### Modules where Compass has greater Depth

- `compass-history` hides immutable Prolly-tree realization, validation,
  publication, leases, and garbage collection behind a cohesive Interface.
- `compass-cypher` plus `compass-query` provides a bounded read-only query
  engine Graphify main does not have.
- `compass-program`, `compass-ir`, and `compass-analysis` establish a typed
  evidence Seam that can deepen without changing graph consumers.
- Native semantic and media Modules enforce size, retry, concurrency, and
  untrusted-input boundaries missing from Graphify main's skill orchestration.

### Seams that need deeper Implementations

- Compatibility needs one machine-readable Module instead of prose/workflow
  duplication.
- Wiki/navigation needs a directional evidence Interface shared by human and
  agent-facing Adapters.
- Current output publication should reuse the staging-and-rename pattern that
  already gives history bundles strong Locality and consistency.
- Qualification should be a release Interface, not a scheduled side workflow.

## Direction options

These are not ranked as bugs and were not promoted into this five-plan batch:

- Add first-class bounded hyperedge read/query APIs before attempting new
  CompassQL syntax.
- Ship a named versioned mixed-corpus profile over existing document, image,
  media, watch, semantic-cache, and history primitives.
- Add a token/trigram natural-query index with cached document frequencies.
- Add a byte-weighted LRU for MCP's multi-project graph cache.
- Evaluate a native Leiden-quality clustering Adapter using deterministic
  quality and stability gates.
- Preserve relation and evidence-site multiplicity in the authoritative graph,
  with a legacy simple-graph compatibility projection.
