# Plan 028: Keep community levels stable across builds and qualify them

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `advisor-plans/README.md` unless a reviewer told you they maintain the index.
>
> **Prerequisites**: Plan 027 must be DONE (`community-hierarchy.json`,
> `crates/compass-graph/src/community/hierarchy.rs`, and the viewer's level
> navigation exist). Plan 026 may land before or after this plan; it does not
> change the artifact.
>
> **Drift check (run first)**: run
> `git diff --stat 3fd246dc..HEAD -- crates/compass-graph/src/community crates/compass-graph/src/cluster.rs crates/compass-core/src/cluster_existing.rs crates/compass-history/src crates/compass-semantic-diff/src crates/compass-output/src/workbench.rs packages/compass-viewer/src/graph tests/viewer scripts/qualify_code_graph_v1.sh docs COMPATIBILITY.md CHANGELOG.md`
> and compare every "Current state" excerpt against the live code. If Plan 027
> landed with different type names or a different artifact shape, adapt the
> steps to the landed names and record the adaptation in your report; if the
> landed shape contradicts a step's intent, STOP.
>
> **Baseline requirement**: the derived community overview
> (`packages/compass-viewer/src/graph/communityOverview.ts`) was written but not
> yet committed when this plan was authored;
> `rg -n "communityOverviewApplies" packages/compass-viewer/src/graph/communityOverview.ts`
> must return a match before Step 4. If it does not, STOP and report. Commit (or
> have the operator commit) that baseline first so the drift check is meaningful.

## Status

- **Priority**: P2 — required before anyone treats hierarchy levels as durable identities

### Implementation status (2026-09-25)

Steps 1–6 are implemented on `codex/community-hierarchy-stability`:
evidence-derived group ids, reconciliation with a persisted identity ledger,
the `compass.community-hierarchy-diff/1` comparison on the history workbench
view, identity-keyed level layouts, and
`./scripts/qualify_code_graph_v1.sh --hierarchy-stability` with every
acceptance entry true and byte-identical reports.

Two adaptations, both recorded in code and docs:

- Reconciliation compares member nodes, but the artifact deliberately stores no
  node lists, so the published partitions are supplied alongside the two
  hierarchies rather than embedded in them.
- The diff is exposed as an additive optional field on the history *workbench
  view* instead of inside `compass.semantic_diff.report/1`. The frozen report
  schema is unchanged, which the STOP conditions require.
- **Effort**: L (reconciliation + events + layout stability + acceptance program)
- **Risk**: MED-HIGH — semantics of group identity and cross-generation comparison
- **Depends on**: `advisor-plans/027-community-hierarchy-artifact.md`
- **Category**: direction / stability / qualification
- **Planned at**: commit `3fd246dc`, 2026-09-23

## Why this matters

A navigation map that reshuffles between builds is worse than no map: readers
lose their place, screenshots and team vocabulary stop matching, and every
downstream integration that keyed on a group id silently invalidates. Community
numeric ids are documented as graph-local, and `labels.json.sig` already lets
Compass reuse labels when a community's member signature is unchanged — but
nothing yet keeps *hierarchy group identity* stable, and nothing states which
groups split, merged, appeared, or disappeared when the code moved.

This plan turns the hierarchy from a per-build artifact into a comparable
series: stable group ids derived from member evidence, an explicit bounded
split/merge event list between generations, level-stable viewer layouts, and a
qualification gate that measures stability instead of asserting it in prose.

## Current state

### Stability primitives that already exist

- `crates/compass-graph/src/cluster.rs:603` — `community_member_signatures`
  returns, per community, a sorted-member SHA-256 truncated to 16 hex
  characters. This is the vocabulary for "the same community".
- `crates/compass-graph/src/cluster.rs:641` — `remap_communities_to_previous`
  remaps a fresh flat partition onto previous numeric ids by counting member
  overlaps. It is used in `crates/compass-core/src/cluster_existing.rs` only on
  the legacy (untyped) path:

```rust
let communities = if previous.is_empty() {
    fresh
} else {
    remap_communities_to_previous(&fresh, &previous)
};
```

- `crates/compass-core/src/cluster_existing.rs` already persists signatures and
  reuses labels when they match:
  `write_python_string_map(staging.join("labels.json.sig"), &signatures)?`
  (`cluster_existing.rs:354`, loaded at `:207`), and the default labeler reuses a saved label only
  when `context.saved_signatures.get(community) == context.signatures.get(community)`
  (`cluster_existing.rs:95`).
- `crates/compass-graph/src/community/incremental.rs` + `cluster_incremental`
  freeze unaffected regions and fall back to a full Leiden run when the affected
  region exceeds `IncrementalClusterLimits::max_affected_nodes` /
  `max_affected_fraction`.
- `crates/compass-graph/src/community/quality.rs` already exports
  `adjusted_rand_index` and `adjusted_mutual_information` — the two metrics a
  stability gate should use between generations.

### Where comparison and history live

- `crates/compass-history/**` stores immutable realizations and manifests;
  `docs/concepts/community-detection.md` states that immutable history "stores
  the sidecar verbatim with its realization", which is why a new artifact needs
  no new storage format.
- `crates/compass-semantic-diff/**` performs evidence-gated comparison of
  historical realizations. Any new split/merge event list must follow its
  evidence and bound rules rather than inventing a parallel comparison engine.
- `crates/compass-output/src/workbench.rs` renders history comparisons
  (`WorkbenchViewContent::History`) and is the only place a human sees them.

### Where the viewer's layout comes from (after Plans 026/027)

- `packages/compass-viewer/src/graph/renderingProfile.ts` seeds deterministic
  positions from node order and the packed-footprint helpers
  (`seedCommunityOverviewPositions`, `packClusterBoxes`/`shelfPack`).
- `packages/compass-viewer/src/graph/communityOverview.ts` orders communities
  (and, after Plan 027, hierarchy groups) by size/importance.
- Positions are recomputed per view and never persisted, so today any ordering
  change moves every bubble even when the group is unchanged.

### Where qualification lives

- `scripts/qualify_code_graph_v1.sh` supports `--fixtures-only`,
  `--community-quality` (runs `crates/compass-graph/examples/community_quality_qualification.rs`
  twice, `cmp`-compares the two reports, then runs a Python acceptance check),
  and `--repositories <manifest>`. Plan 027 adds `--hierarchy`.
- `docs/implementation/community-detection-quality-qualification.md` documents
  the existing community-quality gates; it is the right home for a new
  stability section.

## Commands you will need

```bash
export CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-<your-worktree-name>
test -w "$CARGO_TARGET_DIR" || echo "STOP: workspace volume not mounted or writable"
```

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Graph crate tests | `cargo test -p compass-graph --locked` | all pass |
| Core crate tests | `cargo test -p compass-core --locked` | all pass |
| History tests | `cargo test -p compass-history --locked` | all pass |
| Semantic diff tests | `cargo test -p compass-semantic-diff --locked` | all pass |
| Output tests | `cargo test -p compass-output --locked` | all pass |
| CLI tests | `cargo test -p compass-cli --locked` | all pass |
| Lint | `cargo clippy -p compass-graph -p compass-core -p compass-history -p compass-semantic-diff -p compass-output -p compass-cli --all-targets --locked -- -D warnings` | exit 0 |
| Stability qualification | `./scripts/qualify_code_graph_v1.sh --hierarchy-stability` | every acceptance entry true, identical across two runs |
| Viewer tests | `npm test -w @compass/viewer` | all pass |
| Browser tests | `cd tests/viewer && npx playwright test --project=chromium` | all pass |

## Scope

**In scope**:

- `crates/compass-graph/src/community/hierarchy.rs` (reconciliation + events)
- `crates/compass-graph/src/community/mod.rs`, `crates/compass-graph/src/lib.rs`
- `crates/compass-graph/tests/community_hierarchy.rs`,
  `crates/compass-graph/examples/community_hierarchy_qualification.rs`
- `crates/compass-core/src/cluster_existing.rs` (persist and consume
  `community-hierarchy.json.sig`)
- `crates/compass-semantic-diff/src/**` (bounded hierarchy comparison)
- `crates/compass-output/src/workbench.rs` (surface events in history views)
- `packages/compass-viewer/src/graph/{renderingProfile.ts,communityOverview.ts,CompassGraph.tsx,GraphInspector.tsx}`
  and their tests
- `tests/viewer/**`
- `scripts/qualify_code_graph_v1.sh`
- `docs/concepts/community-detection.md`, `docs/reference/outputs.md`,
  `docs/implementation/community-detection-quality-qualification.md`,
  `COMPATIBILITY.md`, `CHANGELOG.md`
- `crates/compass-output/assets/viewer/*` (regenerated only)

**Out of scope**:

- `graph.json` and any query/CompassQL/MCP surface.
- `crates/compass-history` *storage schema* changes: store the hierarchy
  artifacts and their signature sidecars verbatim, like
  `community-quality.json` is stored today. If you conclude a schema change is
  required, STOP and report.
- Re-tuning Leiden, the topology projection, or the resolution selector.
- Graphify comparison fixtures of any kind.

## Git workflow

- Branch: `codex/community-hierarchy-stability`.
- Conventional, lowercase, imperative commits (match `git log --oneline`).
  Suggested split: `feat(graph): reconcile hierarchy levels across builds`,
  `feat(semantic-diff): report hierarchy split and merge events`,
  `feat(viewer): keep level layouts stable`, `test: qualify hierarchy stability`,
  `docs: document group identity and stability evidence`,
  `build: refresh viewer contract assets`.
- Do NOT push or open a pull request unless the operator instructed it.

## Steps

### Step 1: Derive group ids from member evidence instead of position

In `community/hierarchy.rs`, replace index-derived group ids with evidence
ids: `id = "h<level>-<signature16>"`, where `signature16` is the first 16 hex
characters of a SHA-256 over the group's sorted member-community signatures
(reuse `community_member_signatures`; hash the signatures, not the raw node ids,
so the id survives node-id churn inside a community). Keep `index` purely as
presentation order.

Add a `signature` field per group and per level so two builds can be compared
without expanding membership, and extend the artifact's `identity` block with
the signature algorithm name (for example `hierarchy-signature/v1`). Keep the
payload bound unchanged: level payloads still reference child indices, never
node id lists.

**Verify**: `cargo test -p compass-graph --locked` → existing hierarchy tests
still pass with the new id format.

### Step 2: Reconcile a new hierarchy against the previous one

Add `reconcile_hierarchy(previous: &CommunityHierarchy, next: &mut CommunityHierarchy, policy: &ReconcilePolicy) -> HierarchyReconciliation`:

1. For every level, match groups by member-set overlap (Jaccard over the
   member communities reachable through `child_indices`). Deterministic
   tie-breaks: highest overlap ratio, then larger `member_count`, then the
   lexicographically smaller previous id.
2. A group keeps its previous id when overlap ratio `>= policy.keep_threshold`
   (default 0.5) and the parent match is consistent; otherwise it is a new
   group. Never rewrite a name or evidence block from the previous build.
3. Classify each event against the previous level: `stable` (1:1),
   `split` (one previous group maps to 2+ new groups above the threshold),
   `merged` (2+ previous map to 1 new), `appeared`, `disappeared`. Record exact
   member counts and the overlap ratios that justify each event; when overlap is
   ambiguous (two candidates within `policy.ambiguity_margin`, default 0.05),
   record it as `ambiguous` rather than choosing the most convenient match.
4. Emit `HierarchyReconciliation { matched, split, merged, appeared,
   disappeared, ambiguous, events: Vec<HierarchyEvent> }` with the event list
   bounded (cap at `policy.max_events`, default 256) and exact omitted counts.

Persist the previous state exactly like `labels.json.sig`:
`write_python_string_map(staging.join("community-hierarchy.json.sig"), &signatures)`
in `crates/compass-core/src/cluster_existing.rs`, and load it before building so
reconciliation has a previous hierarchy. Reuse the existing
`cluster_incremental` freeze/fallback rules; reconciliation must never change
partition membership, only identity and reporting.

**Verify**: `cargo test -p compass-graph -p compass-core --locked` → all pass,
including new reconciliation tests.

### Step 3: Report split and merge events for two generations

Add `compass.community-hierarchy-diff/1` produced by comparing two hierarchies
(base and target) inside `crates/compass-semantic-diff` rather than a new crate.
Requirements:

- follow the existing evidence-gate rules: an event is reported only when both
  sides expose the overlapping communities, and ambiguous overlap is reported as
  ambiguous with counts — never resolved by picking a candidate;
- bound the payload (`max_events`, plus exact omitted counts) and make the digest
  deterministic;
- expose it through the existing history comparison path so
  `crates/compass-output/src/workbench.rs` can show a bounded "community
  structure changed" section in history views, and keep it absent when either
  side has no hierarchy artifact (absence means unavailable, not empty);
- do not add a new CLI command in this plan; the existing `compass diff`
  history surface is where the events belong. If that surface cannot carry them
  without a schema change, STOP and report.

**Verify**: `cargo test -p compass-semantic-diff -p compass-output --locked` →
all pass, including a two-generation fixture test and an ambiguous-overlap test.

### Step 4: Keep viewer layouts stable per group identity

In `renderingProfile.ts` / `communityOverview.ts`:

- key seeded positions by group id (not by arrival order or index) so an
  unchanged group keeps its place across rebuilds and across scope switches;
- when a level is entered from a parent group, offset the parent's children to
  orbit the parent position (Plan 027) and reuse the parent level's stored
  positions for the parent bubbles;
- keep the existing deterministic packing guarantees (non-overlapping bubbles,
  label room) and add a test that a reordered-but-equivalent hierarchy produces
  identical positions.

Do not add physics, animation-dependent placement, or persisted browser state.

**Verify**: `npm test -w @compass/viewer` → all pass, including the new
layout-stability test.

### Step 5: Add the stability acceptance gate

Add `--hierarchy-stability` to `scripts/qualify_code_graph_v1.sh`, mirroring the
existing `--community-quality` shape: build a compact fixture repository, apply
a deterministic edit sequence (add symbols to one community, move a file
between two directories, delete a community), rebuild after each edit, and emit
one `compass.community-hierarchy-stability/1` report. Acceptance keys:

- `root_budget_satisfied` and `complete_tree` (from Plan 027);
- `ari_at_least_threshold` / `ami_at_least_threshold` using the existing
  `adjusted_rand_index` / `adjusted_mutual_information` between consecutive
  generations at the root level (record the measured values in the report);
- `stable_ids_for_untouched_groups` — groups whose member set is unchanged keep
  their ids across the sequence;
- `split_merge_events_match_edits` — the reported events correspond exactly to
  the applied edits, with no events for untouched groups;
- `ambiguous_events_reported_not_resolved`;
- `deterministic_digest` — two runs produce byte-identical reports.

Document the thresholds and the fixture edit sequence in
`docs/implementation/community-detection-quality-qualification.md`, and add the
measured numbers to `PERFORMANCE.md` only if the script records timings.

**Verify**: `./scripts/qualify_code_graph_v1.sh --hierarchy-stability` → every
acceptance entry true, identical across two runs.

### Step 6: Documentation and compatibility

- `docs/concepts/community-detection.md`: document group identity (evidence
  ids, member-signature derivation), the reconciliation policy with its
  thresholds, and that ambiguous matches are reported as ambiguous.
- `docs/reference/outputs.md`: document
  `community-hierarchy.json.sig`, the diff schema, and the bounded event lists.
- `COMPATIBILITY.md`: state that both are additive and versioned; unknown majors
  fail; absence means unavailable.
- `CHANGELOG.md`: one release-visible entry covering stable ids and split/merge
  reporting.

**Verify**: `cargo test -p compass-cli --test compass_product --locked` → all
pass; `sh scripts/check_product_boundary.sh` → exit 0.

## Test plan

- `crates/compass-graph/tests/community_hierarchy.rs`: id determinism, keep/new
  classification at the threshold boundary, ambiguous overlap, bounded event
  list with exact omitted counts, and "reconciliation never changes membership".
- `crates/compass-core/src/cluster_existing.rs`: signature sidecar written and
  reused, rename-free reuse across an unchanged rebuild, correct behavior when
  the sidecar is missing or corrupt (must not silently drop identity).
- `crates/compass-semantic-diff`: two-generation diff, absent-side handling,
  ambiguity reporting, payload bounds.
- `crates/compass-output/src/workbench.rs`: bounded history section renders only
  when both sides carry a hierarchy.
- Viewer: position stability across an equivalent-but-reordered hierarchy.
- Qualification: the new `--hierarchy-stability` mode.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --all -- --check` exits 0
- [ ] `cargo test -p compass-graph -p compass-core -p compass-history -p compass-semantic-diff -p compass-output -p compass-cli --locked` exits 0
- [ ] `cargo clippy -p compass-graph -p compass-core -p compass-history -p compass-semantic-diff -p compass-output -p compass-cli --all-targets --locked -- -D warnings` exits 0
- [ ] `./scripts/qualify_code_graph_v1.sh --hierarchy-stability` reports every acceptance entry true and is byte-identical across two runs
- [ ] `npm run typecheck:js` exits 0 and `npm test -w @compass/viewer` exits 0
- [ ] `cd tests/viewer && npx playwright test --project=chromium` exits 0
- [ ] `node scripts/check_viewer_assets.mjs` exits 0
- [ ] `graph.json`, CompassQL, and MCP outputs are unchanged for identical inputs
- [ ] `rg -n "community-hierarchy.json.sig" crates/` shows the sidecar written, loaded, removed with `--no-cluster`, and inventoried
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- Plan 027's artifact shape differs from the excerpts above in a way that
  changes what "a group" is (for example levels referencing node lists).
- Reconciliation appears to require changing partition membership, resolution,
  or the clustering profile.
- Carrying events through the history comparison path requires a
  `compass-history` storage schema change.
- Stability thresholds cannot be met on the compact fixture without lowering
  them; report the measured ARI/AMI values and the fixture edits instead of
  relaxing the gate silently.
- A step's verification fails twice after a reasonable fix attempt.

## Maintenance notes

- Group identity is now a published, evidence-derived id. Any future change to
  the signature algorithm or the reconciliation thresholds is a compatibility-
  sensitive change and needs a version bump plus a MIGRATION note.
- Ambiguity is a first-class outcome: consumers must handle `ambiguous` events
  rather than assuming every previous group has exactly one successor.
- Reviewers should check: ids actually survive an unchanged rebuild, events are
  bounded with exact omitted counts, the diff never promotes a guessed
  correspondence, and the viewer layout test fails if positions key on index
  instead of id.
- Follow-up explicitly deferred: per-level label quality scoring from reader
  tests, and cross-repository comparison of hierarchy shape (the current
  qualification is single-repository).
