# Plan 027: Publish a budgeted community hierarchy for navigation

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `advisor-plans/README.md` unless a reviewer told you they maintain the index.
>
> **Drift check (run first)**: run
> `git diff --stat 3fd246dc..HEAD -- crates/compass-graph/src/community crates/compass-graph/src/lib.rs crates/compass-graph/examples crates/compass-core/src/cluster_existing.rs crates/compass-core/src/pipeline.rs crates/compass-output/src/viewer_model.rs crates/compass-output/src/html.rs crates/compass-output/src/workbench.rs crates/compass-cli/src/lib.rs crates/compass-cli/src/help.rs packages/compass-viewer/src/contracts packages/compass-viewer/src/graph docs COMPATIBILITY.md CHANGELOG.md`
> and compare every "Current state" excerpt against the live code. If a
> community artifact version, viewer-contract version, or clustering entry
> point changed, treat a semantic mismatch as a STOP condition.
>
> **Baseline requirement**: the derived community overview in
> `packages/compass-viewer/src/graph/communityOverview.ts` was written but not
> yet committed when this plan was authored.
> `rg -n "communityOverviewApplies" packages/compass-viewer/src/graph/communityOverview.ts`
> must return a match before Step 6; if it does not, STOP and report. Commit (or
> have the operator commit) that baseline first so the drift check is
> meaningful.

## Status

- **Priority**: P1 — the change that makes repositories with thousands of communities readable

### Implementation status (2026-09-24)

Steps 1–4 and the `hierarchy-json` export are implemented on
`codex/community-hierarchy-artifact`; Steps 5–7 (workbench contract, viewer
level navigation, qualification, and docs beyond the artifact reference) are
outstanding.

Measured on real repositories, relationship-only coarsening cannot bound a
root: 86 of `pallets/flask`'s 112 communities and 2,725 of
`colinhacks/zod`'s 2,781 have no cross-community edge at all, so the group
graph those levels merge is almost empty. This is the plan's STOP condition
("meeting the root budget appears to require merging communities that share no
evidence"). The operator chose to keep the budget and add a second, recorded
rule: levels that relationship evidence cannot reduce are cut from the
directory tree the groups already cite (`locationAffinity`), every level
records its rule and counts, groups that cite no dominant directory are never
merged, and `budgetSatisfied` reports the achieved count. Current results:

| repository | communities | levels | root groups | `budgetSatisfied` |
| --- | --- | --- | --- | --- |
| `pallets/flask` | 112 | 3 | 24 | true |
| `colinhacks/zod` | 2,781 | 4 | 70 (24 directories + 46 location-less singles) | false |
- **Effort**: L (new artifact + graph-side builder + exporter + viewer consumption + qualification)
- **Risk**: MED — a new versioned artifact and a new clustering policy; `graph.json` and query semantics stay unchanged
- **Depends on**: none (Plan 026 is independent; its local label ranking becomes the fallback path)
- **Category**: direction / feature
- **Planned at**: commit `3fd246dc`, 2026-09-23

## Why this matters

Community detection produces a flat partition whose size scales with the
repository. Measured examples: `pallets/flask` yields 174 communities for 4,459
symbols, and `colinhacks/zod` yields 2,641 communities for 58,670 symbols. At
174 units a committed reader can still scan the overview; at 2,641 the map is a
dot field, labels are unreadable, and the community layer is nearly as complex
as the symbol layer it was supposed to summarize. A visual overview needs a
bounded number of *named, nested* units, not a partition sized by modularity
alone.

This plan adds a derived, versioned hierarchy artifact: recursive community
grouping with an explicit per-level budget, evidence-derived group labels with
provenance, and exact coverage accounting. The exporter selects a level instead
of falling back to a flat aggregate, and the viewer navigates levels instead of
one bounded drill-down. Nothing in `graph.json`, CompassQL, or query results
changes.

## Current state

### Where clustering happens

- `crates/compass-graph/src/community/` owns the versioned community subsystem:
  `mod.rs` re-exports `CommunityRequest`, `CommunityProfile`, `CommunityResult`,
  `ResolutionPolicy`, `build_communities`, `CommunityQualityArtifact`,
  `CommunityLimits`, `evaluate_partition_quality`, plus the identity constants
  (`COMPATIBILITY_CLUSTER_ALGORITHM = "seeded-leiden-modularity/v1"`,
  `COMPATIBILITY_CLUSTER_SEED = 42`, `COMPATIBILITY_CLUSTER_LIMITS =
  "community-limits/v1"`, and the matching `QUALITY_CLUSTER_*`).
- `crates/compass-core/src/cluster_existing.rs` is the single build entry point
  for both `compass init` and `compass update` clustering:

```rust
// crates/compass-core/src/cluster_existing.rs:167
let result = build_communities(
    typed,
    &CommunityRequest {
        profile: CommunityProfile::QualityV1,
        resolution: ResolutionPolicy::Fixed(options.resolution),
        exclude_hubs_percentile: options.exclude_hubs,
        previous: (!previous.is_empty()).then_some(&previous),
        incremental: false,
        changed_sources: &changed_sources,
        limits: community_limits,
    },
)?;
```

  and publishes the quality artifact at
  `write_json_atomic(staging.join("community-quality.json"), &artifact, true)?`
  (`cluster_existing.rs:342`).
- `crates/compass-graph/src/cluster.rs` provides the stability and labeling
  primitives this plan builds on: `community_member_signatures` (line 603,
  sorted-member SHA-256 truncated to 16 hex characters),
  `remap_communities_to_previous` (line 641, overlap-count remap),
  `score_communities` / `cohesion_score`, and `label_communities_by_hub`
  (line 412), which produces today's `"<base> (<context>)"` labels.

### Where artifacts are inventoried

A new root artifact must be added to every inventory. Current sites (confirm
with `rg -n "community-quality.json" crates/` before editing):

- `crates/compass-core/src/pipeline.rs:101` — `const ROOT_ARTIFACTS: [&str; 8]`
  (the array length changes to 9);
- `crates/compass-core/src/pipeline.rs:2034` — the `--no-cluster` removal branch;
- `crates/compass-core/src/pipeline.rs:4244` and `:4609`–`:4615` — the
  required-artifact lists;
- `crates/compass-core/src/cluster_existing.rs` — the `artifacts` vector and the
  `BuildGuard::publish_root_artifacts` list around lines 380–410;
- the `manifest.json` writer (find with
  `rg -n "manifest.json" crates/compass-core/src crates/compass-cli/src`).

### Where the viewer consumes models

- `crates/compass-output/src/viewer_model.rs` builds `compass.viewer.graph/1`
  (`GraphViewModel`, `GraphViewNode.member_count`, `GraphViewCommunity`, and the
  fixed `COLORS: [&str; 10]` palette).
- `crates/compass-output/src/html.rs` aggregates above the node limit
  (`fn aggregate`, line ~1113: one node per community, relation text
  `"<count> cross-community edges"`, `confidence: AGGREGATED`) and embeds bounded
  community details (`graph_view_model_bundle_document`).
- `crates/compass-output/src/workbench.rs:158` defines the workbench code view:

```rust
Code {
    model: GraphViewModel,
    community_details: BTreeMap<usize, GraphViewModel>,
},
```

- `crates/compass-cli/src/lib.rs:3281` defines
  `enum ExportViewRequest { Code, Architecture, Call(String), Impact(String), Affected(String), History{..}, Artifact(..) }`;
  `build_export_workbench` (line ~4246) assembles views; the `export` dispatch
  for `html` / `json` / `workbench-json` sits near line 4031, with help text in
  `crates/compass-cli/src/help.rs`.
- `packages/compass-viewer/src/contracts/workbench.ts:33` is a zod
  discriminated union on `kind` (`code`, `call`, `impact`, `architecture`,
  `history`, `affected`, `artifact`).
- `packages/compass-viewer/src/graph/communityOverview.ts` derives a community
  overview in the viewer when a large model arrives unaggregated. Plan 026
  extends that module; this plan makes an exported hierarchy take precedence
  over the derivation.

### Documented guarantees this plan must preserve

From `docs/concepts/community-detection.md`: communities are "deterministic
navigation partitions over a projected view of the Base Graph ... useful
hypotheses about subsystem boundaries, not source truth"; the production profile
is a complete versioned tuple; "exact evidence contributes its full bounded
weight, inferred evidence contributes half, and ambiguous evidence is recorded
as omitted rather than used to invent affinity"; and `community-quality.json` is
"bound to the exact graph generation and canonical `graph.json` digest", with
unknown schemas failing validation and absence meaning "unavailable evidence,
not a quality score of zero".

## Commands you will need

Rust commands MUST set a per-worktree target directory under the workspace
volume, per `AGENTS.md`:

```bash
export CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-<your-worktree-name>
test -w "$CARGO_TARGET_DIR" || echo "STOP: workspace volume not mounted or writable"
```

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Graph crate tests | `cargo test -p compass-graph --locked` | all pass |
| Core crate tests | `cargo test -p compass-core --locked` | all pass |
| Output crate tests | `cargo test -p compass-output --locked` | all pass |
| CLI tests | `cargo test -p compass-cli --locked` | all pass |
| Lint | `cargo clippy -p compass-graph -p compass-core -p compass-output -p compass-cli --all-targets --locked -- -D warnings` | exit 0 |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0 |
| JS typecheck | `npm run typecheck:js` | exit 0 |
| Viewer tests | `npm test -w @compass/viewer` | all pass |
| Browser tests | `cd tests/viewer && npx playwright test --project=chromium` | all pass |
| Rebuild viewer assets | `node scripts/build_viewer_assets.mjs && node scripts/check_viewer_assets.mjs` | manifest matches |
| Hierarchy qualification | `./scripts/qualify_code_graph_v1.sh --hierarchy` | every acceptance entry true, identical across two runs |

## Scope

**In scope**:

- `crates/compass-graph/src/community/hierarchy.rs` (new) and `community/mod.rs`
- `crates/compass-graph/src/lib.rs` (re-exports)
- `crates/compass-graph/tests/community_hierarchy.rs` (new)
- `crates/compass-graph/examples/community_hierarchy_qualification.rs` (new)
- `crates/compass-core/src/cluster_existing.rs`, `crates/compass-core/src/pipeline.rs`
- `crates/compass-output/src/viewer_model.rs`, `html.rs`, `workbench.rs`, `lib.rs`
- `crates/compass-cli/src/lib.rs`, `crates/compass-cli/src/help.rs`,
  `crates/compass-cli/tests/compass_product.rs`
- `packages/compass-viewer/src/contracts/hierarchy.ts` (new),
  `packages/compass-viewer/src/contracts/workbench.ts`, and
  `packages/compass-viewer/src/graph/{communityOverview.ts,CompassGraph.tsx,GraphInspector.tsx,GraphToolbar.tsx,renderingProfile.ts,VisNetworkCanvas.tsx,theme.css}`
- `packages/compass-viewer/src/graph/*.test.*` for the files above
- `tests/viewer/**` (fixtures, specs, `fixtures/generate.ts`)
- `scripts/qualify_code_graph_v1.sh`
- `crates/compass-output/assets/viewer/*` (regenerated only)
- `docs/concepts/community-detection.md`, `docs/reference/outputs.md`,
  `docs/reference/commands.md`,
  `docs/implementation/community-detection-quality-technical-design.md`,
  `COMPATIBILITY.md`, `CHANGELOG.md`

**Out of scope**:

- `graph.json`, `graph-overview.json`, `orientation.json`, and CompassQL/MCP
  result shapes. The hierarchy is additive and must never be required to answer
  a query.
- `crates/compass-history/**` schema changes. History stores published sidecars
  verbatim; store the new sidecar the same way without adding a diff format.
  Cross-generation group identity is Plan 028.
- Language extraction (`compass-languages`, `vendor/`).
- Any Graphify comparison code, fixture, or dependency.

## Git workflow

- Branch: `codex/community-hierarchy-artifact`.
- Conventional, lowercase, imperative commits matching `git log --oneline`
  (for example `fix bounded impact traversal on direct call chains`,
  `build: refresh viewer contract assets`). Suggested split:
  `feat(graph): build budgeted community hierarchy`,
  `feat(core): publish community-hierarchy.json`,
  `feat(output): carry hierarchy levels into the viewer model`,
  `feat(viewer): navigate hierarchy levels`,
  `docs: document hierarchy levels and budgets`,
  `build: refresh viewer contract assets`.
- Do NOT push or open a pull request unless the operator instructed it.

## Steps

### Step 1: Define the hierarchy model and artifact schema

Create `crates/compass-graph/src/community/hierarchy.rs` with:

```rust
pub const COMMUNITY_HIERARCHY_SCHEMA: &str = "compass.community-hierarchy/1";
pub const COMMUNITY_HIERARCHY_BUDGET: &str = "community-hierarchy-budget/v1";

pub struct HierarchyBudget {
    pub root_target: usize,   // default 24
    pub level_target: usize,  // default 300
    pub max_levels: usize,    // default 4, minimum 1
}

pub struct HierarchyLabel {
    pub text: String,
    pub rule: HierarchyLabelRule, // DominantDirectory | ModulePrefix | HubMember | CommunityId
    pub generic: bool,
    pub evidence: BTreeMap<String, serde_json::Value>,
}

pub struct HierarchyGroup {
    pub index: usize,
    pub label: HierarchyLabel,
    pub member_count: usize,
    pub child_indices: Vec<usize>, // indices into the level above
    pub quality: GroupQuality,
}

pub struct HierarchyLevel {
    pub level: usize, // 0 = coarsest
    pub groups: Vec<HierarchyGroup>,
}

pub struct CommunityHierarchy {
    pub budget: HierarchyBudget,
    pub levels: Vec<HierarchyLevel>,
    pub finest_community_count: usize,
    pub finest_signature: String,
    pub budget_satisfied: bool,
    pub digest: String,
}
```

Rules:

- **Levels are defined over the level above, never by repeating node ids.**
  The finest level is the published community partition (ids
  `0..finest_community_count`); `levels[k].groups[*].child_indices` index the
  previous level's groups, and `levels[1]` indexes the finest partition.
  `finest_signature` is the digest of `community_member_signatures` output, so a
  reader can prove the hierarchy matches the published partition without storing
  per-node membership.
- `label.evidence` carries the exact counts that produced the label (for example
  `{"value":"src/flask","coveredMembers":812,"memberCount":903}`).
  `rule == CommunityId` sets `generic: true`; consumers must not depend on
  string comparison.
- `quality` per group: `cohesion` and `conductance` from the existing clustering
  topology, plus `boundary_kinds: BTreeMap<String, usize>` counting members whose
  kind is a boundary kind. Derive that set from the typed kind enum in
  `crates/compass-model/src/code_graph.rs` (`NodeKind::Route`, `Event`,
  `Message`, `Topic`, `Queue`, `Job`, `Resource`, `Schema`, `Query`,
  `Migration`, `ConfigKey`, `Database*`, and the endpoint kinds it defines);
  the viewer already mirrors this set in
  `packages/compass-viewer/src/graph/semanticAppearance.ts` (`BOUNDARY_KINDS`).
  Record the exact kind set used inside the artifact so a reader can audit it.
- Serialize camelCase like the sibling artifact, with a canonical digest over
  the complete payload (mirror `CommunityQualityArtifact::new` in
  `community/artifact.rs`).

**Verify**: `cargo test -p compass-graph --locked` → compiles, existing tests pass.

### Step 2: Build the hierarchy deterministically under a budget

Add `build_community_hierarchy(document, communities, request) -> Result<CommunityHierarchy, CommunityError>`:

1. Start from the published partition. If
   `community_count <= budget.root_target`, return a single-level hierarchy with
   `budget_satisfied = true`.
2. Otherwise coarsen: build the group graph (groups = communities; edge weight =
   number of cross-community relationships from the same typed topology
   projection that clustering uses) and run the same seeded Leiden local moving
   and refinement at a reduced resolution. Choose the resolution by
   deterministic halving from the previous level value (`next = previous / 2`,
   floor `0.05`), stopping at the first value whose group count is
   `<= level_target` (and `<= root_target` for the last level). If the floor
   still exceeds the target, keep the smallest achieved count and set
   `budget_satisfied = false`.
3. Repeat until the count is `<= budget.root_target` or `max_levels` is reached;
   then order levels coarsest-first.
4. Sort groups within a level by `(member_count desc, label text asc, smallest
   member id asc)` and assign `index` from that order, so indices are stable for
   identical input.
5. Prove completeness before returning: children of every group are disjoint,
   their union is exactly `0..previous.group_count`, and
   `sum(child.member_count) == parent.member_count`. On failure return a typed
   `CommunityError` variant (add `IncompleteHierarchy { level, missing }`)
   instead of publishing an inconsistent tree.

Label rule order per group, first rule meeting its threshold wins:
`dominant-directory` (longest common directory prefix covering at least 60% of
the group's member source files) → `module-prefix` (most frequent qualified-name
prefix covering at least 60%) → `hub-member` (today's
`label_communities_by_hub` text) → `community-id` (`"Community <n>"`,
`generic: true`).

Determinism is the contract: identical document plus identical budget must
produce a byte-identical artifact digest. Never merge unrelated groups to hit a
budget and never borrow another group's label evidence.

**Verify**: `cargo test -p compass-graph --locked` after Step 3 lands.

### Step 3: Builder tests

Add `crates/compass-graph/tests/community_hierarchy.rs` (pattern:
`crates/compass-graph/tests/build_coverage.rs` plus the inline tests in
`crates/compass-graph/src/community/build.rs`):

- determinism: build twice from one fixture, assert identical serialized output
  and equal digests;
- budget: a many-small-community fixture produces a root level with at most
  `root_target` groups and `budget_satisfied == true`;
- completeness: every level's children exactly partition the level above and all
  `member_count` sums match;
- label provenance: a fixture with a shared directory yields
  `rule == DominantDirectory` with `coveredMembers` equal to the counted files; a
  mixed group falls back in order and the last fallback is `generic: true`;
- honesty: when the budget cannot be met, the artifact records the achieved
  count and `budget_satisfied == false`;
- limits: `max_levels`, `level_target`, and the resolution schedule appear in the
  artifact identity/limits block, and exceeding a limit fails with the typed
  error instead of truncating.

**Verify**: `cargo test -p compass-graph --locked` → all pass, including the new
integration test file.

### Step 4: Publish the artifact from the build pipeline

In `crates/compass-core/src/cluster_existing.rs`:

- build the hierarchy immediately after `build_communities` succeeds, from the
  same document, options, limits, generation id, and graph identity used for
  `CommunityQualityArtifact::new`;
- write `community-hierarchy.json` atomically beside `community-quality.json`,
  add it to the `artifacts` vector and the `BuildGuard::publish_root_artifacts`
  list;
- when `options.no_cluster` is set, remove a stale `community-hierarchy.json`
  exactly like the quality artifact is removed.

In `crates/compass-core/src/pipeline.rs`:

- extend `ROOT_ARTIFACTS` to 9 entries, preserving the existing ordering style;
- extend the `--no-cluster` removal branch and the required-artifact lists;
- update the `manifest.json` writer so the new artifact appears with its digest
  and byte size.

The hierarchy must not become a build prerequisite. If hierarchy construction
fails with a limit error, `graph.json` and `community-quality.json` must still
publish, and the omission must be explicit. Follow the behavior
`community-quality.json` uses for an equivalent condition; if no such path
exists, STOP and report rather than inventing one.

**Verify**: `cargo test -p compass-core --locked` → all pass, including a new
test asserting the artifact is published, its digest matches its payload, and an
unchanged rebuild does not rewrite it.

### Step 5: Expose the hierarchy to exports and the viewer contract

- `crates/compass-cli/src/help.rs` and `lib.rs`: add
  `compass export hierarchy-json [--graph PATH] [--output PATH]`, which validates
  the artifact (unknown major version → typed error, non-zero exit) and emits it
  unchanged. Add `--hierarchy-level N` to `compass export html`, defaulting to
  level 0, with an explicit bounded error when the artifact is absent or the
  level does not exist.
- `crates/compass-output/src/workbench.rs`: extend the code view with an optional
  hierarchy payload (`hierarchy: Option<CommunityHierarchyView>`), carrying level
  metadata plus one `compass.viewer.graph/1` projection per level, with the bound
  recorded in `WorkbenchCoverage`.
- Standalone `graph.html`: embed the hierarchy beside the existing model script
  (for example `<script id="compass-viewer-hierarchy" type="application/json">`)
  exactly the way community details are embedded today, keeping the document
  self-contained and offline.
- `packages/compass-viewer/src/contracts/hierarchy.ts` (new): zod schema for
  `compass.community-hierarchy/1` that rejects unknown majors, plus the
  view-model payload; extend `contracts/workbench.ts` with the optional
  hierarchy on the `code` view.

**Verify**: `cargo test -p compass-output -p compass-cli --locked` → all pass;
`npm run typecheck:js` → exit 0.

### Step 6: Navigate levels in the viewer

- `CompassGraph.tsx`: add `activeLevel` (0 = root) and turn the Plan 026
  breadcrumb into `Repository ▸ Level 0 group ▸ Level 1 group`. Double-click a
  group descends one level; `Escape`, the breadcrumb, and the existing
  **Overview** button ascend one level. Search still reaches every symbol and
  opens the deepest available level with the symbol focused.
- `communityOverview.ts`: when an exported hierarchy is present, use its groups
  and per-level edges instead of deriving a flat overview; keep the current
  derivation (and the Plan 026 relationship mix) as the fallback for models
  without a hierarchy.
- `renderingProfile.ts`: seed positions per level so children orbit their
  parent's position — descending a level must read as a spatial zoom, not a
  jump. Reuse the existing deterministic packing helpers; do not add physics to
  the root level.
- `GraphInspector.tsx`: show the selected group's cohesion, top boundary kinds,
  child count, and shown/omitted counts when the panel bound applies.
- `theme.css`: reuse the existing scope-toggle and breadcrumb styles; no new
  colour scale. The scope toggle reads `Level 0 | Level 1 | ... | Symbols`,
  bounded to the artifact's levels (at most 4) plus `Symbols`.

**Verify**: `npm test -w @compass/viewer` → all pass;
`cd tests/viewer && npx playwright test --project=chromium` → all pass.

### Step 7: Qualification, docs, and compatibility

- Add `crates/compass-graph/examples/community_hierarchy_qualification.rs` and a
  `--hierarchy` mode to `scripts/qualify_code_graph_v1.sh`, mirroring the
  existing `--community-quality` mode (two runs, `cmp` byte equality, then a
  Python acceptance check). Acceptance keys must include
  `root_budget_satisfied`, `complete_tree`, `labels_have_provenance`,
  `generic_root_labels_bounded` (for example at most 25% of root groups are
  generic), `deterministic_digest`, and `bounded_levels`.
- Extend `crates/compass-cli/tests/compass_product.rs` with one case per new
  surface: `export hierarchy-json` reproduces the published artifact, and an
  unknown-major artifact exits non-zero with a typed message.
- Documentation: `docs/concepts/community-detection.md` (new "Hierarchy levels
  and budgets" section documenting the budget tuple, label rules with
  provenance, completeness proof, and the hypothesis framing),
  `docs/reference/outputs.md` (artifact table entry plus viewer level behavior),
  `docs/reference/commands.md` and `help.rs` (new export),
  `docs/implementation/community-detection-quality-technical-design.md`
  (implementation notes), `COMPATIBILITY.md` (additive artifact: absence means
  unavailable, unknown major fails), and `CHANGELOG.md` (release-visible entry).
- `docs/reference/outputs.md` must state that level membership is a navigation
  aid derived from the same evidence as communities, that ids are graph-local,
  and that the artifact never changes node, edge, or query results.

**Verify**: `./scripts/qualify_code_graph_v1.sh --hierarchy` → every acceptance
entry true, identical report across two runs;
`cargo test -p compass-cli --test compass_product --locked` → all pass.

## Test plan

- `crates/compass-graph/tests/community_hierarchy.rs`: determinism, budget,
  completeness, label provenance and fallback order, honesty when the budget is
  unmet, limits.
- Inline tests in `community/hierarchy.rs`: digest stability, typed errors,
  empty-graph and single-community degenerate cases.
- `crates/compass-core/src/cluster_existing.rs` tests: artifact published,
  digest-bound, stable across an unchanged rebuild, removed under `--no-cluster`.
- `crates/compass-output` tests: hierarchy payload embedded when present, absent
  otherwise, bounded per level.
- `crates/compass-cli/tests/compass_product.rs`: new export surface plus version
  rejection.
- Viewer: `communityOverview.test.ts` (artifact hierarchy preferred over
  derivation), `CompassGraph.community.test.tsx` (descend, ascend, breadcrumb),
  and a new `tests/viewer/hierarchy.spec.ts` browser spec on a generated fixture.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --all -- --check` exits 0
- [ ] `cargo test -p compass-graph -p compass-core -p compass-output -p compass-cli --locked` exits 0 with the new tests present
- [ ] `cargo clippy -p compass-graph -p compass-core -p compass-output -p compass-cli --all-targets --locked -- -D warnings` exits 0
- [ ] `npm run typecheck:js` exits 0
- [ ] `npm test -w @compass/viewer` exits 0
- [ ] `cd tests/viewer && npx playwright test --project=chromium` exits 0
- [ ] `./scripts/qualify_code_graph_v1.sh --hierarchy` reports every acceptance entry true and is byte-identical across two runs
- [ ] `rg -n "community-hierarchy.json" crates/` lists the artifact in every inventory site (`ROOT_ARTIFACTS`, staged artifacts, publish list, removal branch, required list, manifest)
- [ ] `graph.json`, `graph-overview.json`, and CompassQL/MCP outputs are unchanged for identical inputs (`git diff` on those writers is empty)
- [ ] `node scripts/check_viewer_assets.mjs` exits 0
- [ ] `sh scripts/check_product_boundary.sh` exits 0
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- `build_communities`, `CommunityRequest`, or `CommunityQualityArtifact::new` no
  longer match the shapes quoted in "Current state".
- Coarsening cannot produce a stable partition for a fixture (for example a star
  group graph whose centre merges everything) — report the fixture and achieved
  counts instead of special-casing it.
- Meeting the root budget appears to require merging communities that share no
  evidence, or dropping members.
- Publishing the artifact would require changing `graph.json`, the workbench
  schema version, or an existing artifact's fields.
- The `manifest.json` writer cannot represent a new artifact without a schema
  change — report before inventing a field.
- A step's verification fails twice after a reasonable fix attempt.

## Maintenance notes

- Any change to the clustering profile (algorithm, topology, seed, limits,
  resolution policy) must re-derive the hierarchy: the artifact identity records
  the profile, and a mismatch must fail validation rather than render a
  mixed-revision hierarchy.
- Level membership must stay derivable from `child_indices` alone. If a future
  feature needs per-node level assignment, add a projection to the viewer or a
  query surface instead of embedding node id lists in the artifact.
- Reviewers should check: the completeness proof, deterministic digest, label
  provenance for every group including the generic fallback, bounded payload on
  a zod-class repository (roughly 2,600 finest communities must stay well under
  a megabyte), and that no query path consults the hierarchy.
- Follow-up explicitly deferred: stable group identity across builds and
  split/merge reporting (Plan 028), plus per-level label budgets tuned against
  real repository reading tests.
