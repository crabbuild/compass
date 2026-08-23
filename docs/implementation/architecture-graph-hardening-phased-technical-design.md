# Architecture graph hardening phased technical design

Status: implemented and qualified on 2026-08-23; see the
[qualification report](architecture-graph-hardening-qualification.md)

Scope: project-specific architecture projection, architecture-view contracts,
and shared viewer presentation

Supersedes: the section-derivation, edge-counting, and `Other` overflow parts of
[VS Code Architecture Flow Design](../superpowers/specs/2026-07-28-vscode-architecture-flow-design.md)

Preserves: that design's bounded CLI export, extension-host indexing, paged
host/webview messages, local SVG rendering, accessibility, and cancellation
model

## Overview

Compass should present architecture derived from the project's source evidence,
ownership, and relationships. It should not translate arbitrary repositories
into a fixed catalog of product archetypes, accept repeated low-information
labels, or represent every omitted community as one connected `Other`
subsystem.

The baseline architecture path had four independent correctness problems:

1. sections are selected and named from the complete graph before the viewer's
   Production filter is applied;
2. source scope relies primarily on path strings and misses tracked generated
   bundles that do not live under conventional build directories;
3. every graph relationship is counted and rendered as a call, including
   containment and broad references; and
4. a flat top-N community list turns every omitted community into one false
   `Other` subsystem.

Replacing predefined archetype names with community labels is a useful tactical
correction, but it cannot solve these deeper issues. Repeated labels such as
`Crates Compass Output` and a dominant `Other` node are expected while grouping,
source scope, relationship semantics, and naming quality remain shallow and
distributed across Rust and TypeScript callers.

This design deepens one architecture projection module in `compass-output`:

```text
validated compass.graph/1 + communities + labels + optional overlay
                              |
                              v
                 architecture_projection
          source evidence | relation policy | hierarchy
          ranking | naming | stable identity | quality
                              |
                              v
              compass.viewer.architecture/1
                              |
                 +------------+-------------+
                 |                          |
                 v                          v
          standalone workbench       VS Code host index
                 |                          |
                 +------------+-------------+
                              v
                  shared React architecture view
```

The module has a small interface and a large implementation, providing leverage
to every renderer and locality for future architecture-quality fixes. Deleting
it would force source classification, relationship interpretation, grouping,
naming, omissions, and quality logic back into every caller; it therefore
passes the deletion test.

## Graphify findings and Compass position

Graphify is research input, not a compatibility oracle. Its current call-flow
renderer uses predefined section archetypes and an `Other` overflow bucket, so
Compass must not copy that presentation behavior. Useful Graphify ideas live in
its clustering and edge-selection implementation:

- seeded Leiden/Louvain-style community detection;
- optional hub exclusion and deterministic reattachment;
- oversized and low-cohesion community splitting;
- stable ordering and membership-signature-protected labels; and
- preference for calls, uses, methods, and imports over structural edges.

Compass already implements deterministic Louvain clustering, hub exclusion,
oversized and low-cohesion splitting, stable community remapping, source-aware
hub labels, and community membership signatures in `compass-graph`. The plan
reuses and deepens those native implementations. It adds no Graphify runtime,
test, configuration, artifact, repository, or fallback dependency.

Primary research references:

- [Graphify architecture](https://github.com/Graphify-Labs/graphify/blob/v8/ARCHITECTURE.md)
- [Graphify call-flow implementation](https://raw.githubusercontent.com/Graphify-Labs/graphify/v8/graphify/callflow_html.py)
- [Graphify clustering implementation](https://raw.githubusercontent.com/Graphify-Labs/graphify/v8/graphify/cluster.py)
- [Graphify source detection](https://raw.githubusercontent.com/Graphify-Labs/graphify/v8/graphify/detect.py)

## Goals

- Derive Production architecture before grouping, naming, ranking, or routing.
- Keep tests, generated code, vendor code, documentation, and unknown-source records reachable
  through explicit All-code scope without allowing them to shape Production.
- Give each represented relationship an explicit architecture relation class.
- Stop labeling containment, references, and other structural edges as calls.
- Replace flat top-N plus `Other` with a bounded hierarchical overview and
  exact omission metadata.
- Produce deterministic, project-specific, unique subsystem names with evidence
  provenance and a quality score.
- Preserve community identity independently from display names.
- Publish architecture quality separately from extraction completion.
- Keep all work local, deterministic, bounded, source-grounded, and portable.
- Use one Rust-owned semantic projection for HTML, workbench JSON, and VS Code.
- Preserve complete search, paging, evidence inspection, and source navigation.

## Non-goals

- Change graph extraction, graph node identity, edge identity, direction,
  multiplicity, or provenance.
- Replace `compass-graph` clustering with a provider or external library.
- Require model credentials, embeddings, a vector database, or network access.
- Claim that one clustering realization is the only correct architecture.
- Hide tests, generated code, vendor code, documentation, or unknown-source records from the
  underlying graph or All-code exploration.
- Render all communities or symbols simultaneously.
- Infer package ownership from the live filesystem when exporting an immutable
  historical graph.
- Add an extension-host or React implementation of architecture semantics.
- Treat Graphify agreement as an accuracy measurement.

## Required invariants

### Evidence and scope

- Production grouping sees only nodes classified as Production.
- All-code grouping is derived independently; it is not Production plus one
  synthetic remainder group.
- Generated and vendor nodes in the Production projection are always zero.
- Classification retains a reason and never silently converts missing evidence
  into Production.
- Historical projection uses evidence inside the selected graph and its
  explicit overlay; it never inspects a different current checkout.
- A source-classification limit is an explicit error or degraded diagnostic,
  never an empty classification.

### Relationship meaning

- Original relation names, direction, multiplicity, confidence, source anchors,
  and provenance remain available in detail records.
- The architecture relation class is a presentation projection; it does not
  rewrite the graph edge.
- Unknown relation names are retained as `unknown` and excluded from the
  default topology until a versioned policy classifies them.
- Structural relationships never contribute to a count labeled `calls`.
- Every admitted relationship is internal, cross-group, or unassigned within
  its scope and lens; the three counts sum to the admitted total.

### Grouping and identity

- A node belongs to at most one leaf group in one scope realization.
- Group ownership and community membership are deterministic for equivalent
  inputs.
- Display labels are not identity.
- No renderer creates a connected `Other` group.
- Omitted groups and routes retain exact represented and omitted counts.
- Group and route ordering is explicit and stable.
- An explicit project overlay cannot overlap selectors silently.

### Boundedness and failure

- Overview groups, routes, candidates per name, representative members, and
  diagnostics all have hard limits.
- Truncation retains required/shown/omitted counts and the active limit.
- Invalid configuration, unknown contract majors, duplicate explicit IDs, and
  overlapping ownership fail before publication.
- Projection either publishes one coherent validated model or no model.

## Ownership and module depth

| Module | Interface ownership | Implementation hidden behind the seam |
| --- | --- | --- |
| `compass-files` | generated-source evidence used in `FileRecord.generated` | bounded marker reads, deterministic path rules, admitted VCS metadata |
| `compass-graph` | communities, cohesion, connectivity, stable remapping, member signatures | topology construction and clustering implementation |
| `compass-output::architecture_projection` | one validated architecture projection interface | source scope, relation classes, ownership hierarchy, ranking, naming, IDs, omissions, diagnostics |
| `compass-cli` | arguments, streams, exits, capability advertisement | thin adapter to `compass-output` |
| shared viewer | typed architecture rendering and interaction | map layout, filters, inspector, accessibility |
| VS Code host | bounded retained index and paging | process capture, validation, request identity, pages |

`compass-output::architecture_projection` is the new deep module. The viewer
must not repeat relation classification, source classification, group naming,
or omission policy. The VS Code host may filter, aggregate, index, sort, and
page typed fields already supplied by Compass, but it must not infer new
architecture meaning.

## Selected module interface

The external Rust seam has one projection operation:

```rust
pub struct ArchitectureProjectionInput<'a> {
    pub document: &'a GraphDocument,
    pub communities: &'a Communities,
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub overlay: Option<&'a ArchitectureOverlay>,
    pub project_name: &'a str,
}

pub struct ArchitectureProjectionOptions {
    pub scopes: BTreeSet<ArchitectureScope>,
    pub default_lens: ArchitectureLens,
    pub limits: ArchitectureProjectionLimits,
}

pub fn project_architecture(
    input: ArchitectureProjectionInput<'_>,
    options: &ArchitectureProjectionOptions,
) -> Result<ArchitectureViewModel, ArchitectureProjectionError>;
```

The interface invariants are:

- inputs are already validated Compass graph and community records;
- `scopes` must contain Production and may contain All-code;
- limits are checked before large reservations and during candidate generation;
- output is canonically ordered and self-validating;
- errors distinguish invalid inputs, invalid overlay, exceeded limits, missing
  production evidence, and internal invariant violations; and
- optional provider labels arrive only through `community_labels`; the module
  never contacts a provider.

Internal seams are private implementation details. Introduce an adapter only
when two implementations exist. Source rules, relation policy, naming, and
ranking begin as closed deterministic tables/functions rather than trait
families with one hypothetical adapter.

## Projection pipeline

```text
1. Index graph evidence
   node ID -> node
   normalized source path -> FileRecord
   node ID -> community
   edge endpoints -> validated nodes

2. Classify node source scope
   explicit overlay -> graph generated evidence -> conservative path evidence
   -> production/unknown

3. Build independent scope realizations
   Production
   All code

4. Classify relationships
   execution | dependency | type | structure | contextual | unknown

5. Resolve hierarchy and leaf groups
   explicit owner -> source-backed package/module owner -> path owner
   -> graph community

6. Rank groups and routes
   represented production members + cross-group signal + connectivity + cohesion

7. Generate and validate display names
   overlay -> signature-valid persisted label -> owner -> path -> declarations
   -> hub fallback

8. Apply overview bounds
   selected groups/routes + exact omission metadata; never `Other`

9. Compute quality and validate invariants

10. Canonically encode the architecture model
```

## Source-scope evidence

### Classification inputs

Classification uses only deterministic evidence available in the graph or
explicit overlay:

1. an explicit project overlay scoped to a normalized path or package;
2. `GraphMetadata.files[*].generated` for the node's source path;
3. admitted generated-source evidence produced during discovery;
4. conventional vendor and third-party path segments;
5. conventional test directories and filename forms;
6. conventional generated/build path segments and generated filename forms;
7. a non-empty recognized source path; or
8. missing/unsupported source evidence.

Each decision records a stable reason such as `overlay`, `graph_generated`,
`vendor_path`, `test_path`, `generated_path`, `source_path`, or
`missing_source`.

The current 2,048-byte generated-marker scan in `compass-core` remains useful
but insufficient. `compass-files` should deepen generated evidence by admitting
bounded, deterministic project metadata such as recognized generated markers
and VCS-generated attributes. Content-shape guesses such as “long line means
generated” are not admitted because they can misclassify legitimate sources.
Minified suffixes may be admitted only through an explicit, documented filename
rule with negative fixtures.

### Precedence

Explicit overlay has highest precedence. Vendor evidence precedes test or
generated subpaths inside vendored trees because the code's project ownership
is vendor. Generated evidence precedes test evidence for generated test
fixtures. A missing path is Unknown, not Production.

The Production realization includes only Production. All-code retains every
scope and exposes exact counts for Production, Test, Generated, Vendor,
Documentation, and Unknown.

## Architecture relationship policy

The projection retains every original relationship and assigns one closed
class:

| Class | Intended meaning | Default overview |
| --- | --- | --- |
| `execution` | runtime invocation, routing, handling, mounting, data production/consumption | included |
| `dependency` | imports, exports, declared dependencies, configuration dependencies | included |
| `type` | extends, implements, overrides, type/return/instantiation relationships | optional lens |
| `structure` | contains, declares, ownership, documentation structure | excluded |
| `contextual` | tests, documents, aliases, broad references, derivation | excluded |
| `unknown` | relation not admitted by the current policy | excluded and diagnosed |

Before implementation, Phase 0 freezes the complete relation vocabulary from
the public graph model and assigns every known relation in one reviewable table.
The default Architecture lens combines execution and dependency while retaining
their individual counts. Separate Execution, Dependency, Type, and Structure
lenses remain available.

User-facing terminology changes from `calls` to `relationships` except inside
the Execution lens, where exact `calls` records may still be labeled calls.
Route thickness uses a capped deterministic scale over admitted relationship
count. The inspector shows the relation-class mix and the original relation
names behind a route.

## Hierarchical architecture and omission

### Depth model

The overview has at most two visible group depths:

```text
project/workspace
    owner group: crate, package, module root, or top-level source owner
        leaf group: one or more graph communities
```

Ownership evidence is selected in this order:

1. explicit overlay ownership;
2. source-backed package/workspace/module nodes and their exact ownership or
   containment relationships;
3. a dominant normalized source prefix after generic segments such as `src`,
   `lib`, `crates`, and `packages` are removed from display candidates; and
4. graph community as the leaf fallback.

Communities remain the topology implementation. The hierarchy is a
presentation projection and does not rewrite community assignments.

### Ranking

Raw member count is not sufficient. Group ranking uses an explicit tuple:

1. descending represented Production nodes;
2. descending admitted cross-group relationship count;
3. descending distinct neighboring groups;
4. descending cohesion/connectivity score;
5. stable group ID.

Generated, Test, Vendor, and Unknown counts do not increase Production rank.
All-code ranking uses the same tuple within its realization and discloses scope
composition.

### Removing `Other`

When overview limits are reached, omitted groups are not merged. The model
publishes:

- total, shown, and omitted group counts;
- represented and omitted node counts by source scope;
- represented and omitted relationships by class;
- strongest omitted group names/IDs up to a separate bounded witness limit;
- routes omitted because one or both endpoint groups are not shown; and
- the exact limits that caused omission.

Every omitted group remains reachable through the directory, search, or
focused drill-down. No route may target an omission summary.

## Naming and stable identity

### Identity

Automatic group IDs use stable owner identity plus graph community identity,
not display text. Existing `remap_communities_to_previous` continuity is
preserved. Explicit overlay groups use validated project-owned IDs. A display
name change never changes selection, saved layout, route identity, or history
matching.

### Candidate order

The naming implementation generates bounded candidates in this order:

1. explicit overlay name;
2. persisted label whose membership signature matches;
3. source-backed package, crate, or module owner;
4. dominant meaningful relative source prefix;
5. representative declarations selected by connectivity and semantic kind;
6. deterministic community hub label with source context; and
7. transparent fallback `Unnamed subsystem · <owner/path>`.

Provider labels are optional persisted labels. They may rename a fixed member
set but never select members, source scope, relation class, hierarchy, or rank.
Their provenance is `provider` and their membership signature is mandatory.

### Quality gate

A candidate is rejected when it is empty, generic, dominated by repository
scaffolding words, minified/low-entropy, an opaque ID, misleadingly identical to
another group, or unsupported by the group's source evidence. Generic tokens
include language-neutral scaffolding such as `src`, `lib`, `crates`, `packages`,
`module`, `index`, `main`, and the project name when it adds no distinction.

Duplicate candidates are disambiguated with owner/path evidence before a
community identifier is used. Every accepted name publishes:

- display value;
- provenance: `overlay`, `persisted`, `owner`, `path`, `declaration`, `hub`,
  `provider`, or `fallback`;
- membership signature;
- bounded supporting evidence; and
- deterministic quality score and rejection diagnostics.

## Architecture quality contract

Extraction completion and architecture quality are separate facts. The header
may show both, for example:

```text
Extraction: complete     Architecture projection: degraded
```

The architecture model publishes:

```rust
pub enum ArchitectureQualityStatus {
    Good,
    Degraded,
    Insufficient,
}

pub struct ArchitectureQuality {
    pub status: ArchitectureQualityStatus,
    pub metrics: ArchitectureQualityMetrics,
    pub diagnostics: Vec<ArchitectureQualityDiagnostic>,
}
```

Required metrics include:

- source-scoped node counts and Unknown fraction;
- generated/vendor leakage into Production, which must be zero;
- represented and omitted node/relationship fractions;
- duplicate and fallback label counts;
- largest leaf-group share;
- admitted relationship counts by class;
- unknown relation count;
- unassigned node and relationship counts; and
- group identity/name churn when a previous realization is available.

`Insufficient` means no source-grounded Production projection can be formed.
`Degraded` means the projection exists but violates a calibrated quality gate
or has material omissions. `Good` means all hard invariants pass and calibrated
coverage gates pass. Phase 0 records real-repository distributions before
numeric thresholds are frozen; thresholds then become named versioned policy
constants with tests, not UI magic numbers.

Diagnostics have stable codes, severity, observed value, threshold when
applicable, exact witnesses, and one recommended action. Examples include
`generated_scope_leak`, `unknown_source_share`, `duplicate_group_name`,
`dominant_group`, `overview_omission`, `unknown_relation`, and
`unassigned_relationship`.

## Machine contract

The semantic change is not additive to `compass.viewer.callflow/1`. Counts no
longer mean every graph edge is a call, scope realizations are independent, and
flat sections become hierarchical groups. The rollout introduces:

```text
compass.viewer.architecture/1
```

The top-level shape contains:

```text
schema
title
nodes[]                 full bounded detail inventory, each with source scope
relationships[]         original relation + architecture class + evidence
projections[]           Production and optional All-code realizations
  scope
  groups[]              stable ID, parent, name evidence, rank, counts
  routes[]              endpoint IDs and relation-class/evidence counts
  omissions
  quality
statistics
provenance
limits
```

Nodes and relationships are stored once. Scope realizations reference stable
IDs and publish summaries; this avoids duplicating complete detail arrays.
Consumer schemas reject unknown majors and dangling group, node, route, or
parent references.

`compass export callflow-json` remains a compatibility-sensitive command. At
the coordinated cutover it emits the architecture contract and advertises it
through capabilities; the CLI help explains the new schema. A clearer
`architecture-json` alias may be considered separately, but command renaming is
not required for this design and must not delay correctness.

The outer `compass.viewer.workbench/1` contract remains unchanged only if its
architecture content is explicitly schema-tagged and opaque at that seam. Phase
4 audits strict consumers; if they validate the nested call-flow shape as part
of the outer contract, the workbench major changes in the same coordinated
cutover.

## Optional project architecture overlay

Automatic projection remains the zero-configuration default. The existing
`--sections <PATH>` adapter is deepened after automatic quality is shipped into
a versioned architecture overlay that can:

- assign stable owner/group IDs by exact package, normalized path, or community;
- supply project vocabulary for names;
- combine or split selected automatic groups within declared bounds;
- classify explicit generated/vendor/test paths; and
- pin overview visibility without inventing relationships.

Precedence is explicit CLI overlay, project `.compass` overlay, then automatic
projection. Selectors must be deterministic, non-overlapping, and complete
within each explicit group. Invalid selectors, duplicate IDs, ambiguous
packages, and overlaps fail with source-located diagnostics.

The first overlay version does not execute user code, glob outside the project,
or select nodes by free-form query. It cannot alter graph edges, confidence,
provenance, or source anchors.

## Presentation behavior

- Production remains the initial scope.
- Architecture and Extraction statuses are visually separate.
- The default lens is Architecture, combining execution and dependency.
- The toolbar offers Execution, Dependency, Type, Structure, and All relations.
- Overview labels show project-specific names and optional owner context.
- Omitted coverage appears as disclosure text and a directory action, never a
  map node.
- Selecting an owner drills into its leaf groups; Back restores the already
  loaded overview.
- Route labels and counts use `relationships` unless the active lens is exact
  calls.
- The inspector shows relation mix, source-scope composition, name provenance,
  quality diagnostics, and bounded supporting evidence.
- Generated, Vendor, Test, Documentation, and Unknown remain visible in All-code scope and
  search.
- Accessible table alternatives expose the same hierarchy, routes, omissions,
  and diagnostics as the map.

## Failure behavior

| Condition | Result |
| --- | --- |
| empty graph | existing explicit empty-graph error |
| no Production nodes | publish All-code only if requested; Production quality is Insufficient |
| missing file inventory | classify from conservative node evidence; quality is Degraded with exact count |
| generated evidence conflict | explicit overlay wins; otherwise exclude from Production and diagnose |
| unknown relation | retain detail, exclude from default topology, diagnose |
| invalid overlay | fail before model publication |
| hierarchy limit exceeded | publish bounded groups plus exact omissions |
| naming candidate limit exceeded | use deterministic fallback and diagnose; never loop unboundedly |
| dangling endpoint/group reference | fail validation and publish no model |
| unsupported contract major | consumer rejects with upgrade guidance |

## Compatibility and migration

This is a coordinated machine-contract cutover, not a silent reinterpretation
of `compass.viewer.callflow/1`.

The cutover phase must include:

1. native Rust contract tests;
2. TypeScript runtime schema and fixture updates;
3. CLI capability and compatibility updates;
4. VS Code minimum-version/capability handling;
5. reference documentation updates;
6. a `MIGRATION.md` entry for direct call-flow JSON consumers; and
7. a release-visible `CHANGELOG.md` entry.

No graph artifact migration is required because architecture is derived from
the existing graph. Historical graphs remain immutable. Regenerating the
architecture projection with a newer binary may change grouping, names, and
quality diagnostics; the selected graph's node and edge identities remain
unchanged.

## Phased delivery plan

Each phase is independently reviewable and stops when its acceptance criteria
fail. Early phases build and characterize the deep module without changing the
public contract. Phase 4 is the coordinated contract cutover. Later phases are
additive to the new architecture contract.

| Phase | Shippable result | Depends on | Public effect |
| --- | --- | --- | --- |
| 0 | reproducible quality baseline and regression corpus | none | none |
| 1 | private source-correct projection module | 0 | generated evidence may improve graph inventory |
| 2 | private typed relationship topology | 1 | none |
| 3 | complete private hierarchy, naming, omissions, and quality model | 2 | none |
| 4 | coordinated architecture contract cutover | 3 | machine contract and consumer migration |
| 5 | hierarchical map and quality UX | 4 | user-visible architecture experience |
| 6 | optional project overlay and qualification evidence | 5 | additive configuration and documentation |

Every production commit includes its lowest-seam regression tests. Keep policy
changes separate from mechanical moves, and keep generated viewer assets in the
same commit as the source change that produced them.

### Phase 0: Freeze current behavior and quality baselines

Purpose: make the screenshot failure reproducible before moving ownership.

Deliverables:

- Add graph fixtures for repeated path-token labels, one tracked generated
  bundle, vendor/test communities, containment-dominated topology, thousands of
  thin communities, and duplicate community labels.
- Add a real-repository qualification snapshot for Compass itself, including
  exact node/edge/community/source-scope distributions.
- Record the complete public relationship vocabulary and proposed relation
  class in a reviewable test table.
- Add characterization tests for current `Other` size, duplicate names,
  generated leakage, all-edge-as-call counts, and ordering.
- Define the quality metric formulas and capture baseline values without yet
  enforcing numeric status thresholds.

Primary files:

- `crates/compass-output/tests/callflow_model.rs`
- new `crates/compass-output/tests/architecture_projection_fixtures.rs`
- `tests/viewer/`
- `scripts/qualify_code_graph_v1.sh` or a focused architecture-quality runner
- reviewable fixtures under `fixtures/`

Acceptance:

- The current Compass reproduction fails at least the generated-leakage,
  duplicate-label, dominant-group, and relation-semantics checks.
- Fixture permutations produce identical recorded ordering and metrics.
- No production behavior or public schema changes.

Suggested commit sequence:

1. Add synthetic failure fixtures and current-behavior characterization.
2. Add metric formulas and deterministic permutation tests.
3. Add the pinned Compass qualification case and baseline record.

Verification:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo test -p compass-output --locked
npm run test:js
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  ./scripts/qualify_code_graph_v1.sh --fixtures-only
```

Rollback: test and fixture commits are independently revertible. Do not proceed
if the reproduction cannot distinguish tactical naming improvements from the
deeper failure.

### Phase 1: Deepen source evidence and projection ownership

Purpose: create the architecture projection seam and ensure Production is an
input to grouping rather than a UI filter.

Deliverables:

- Create `crates/compass-output/src/architecture_projection/` with the selected
  input, options, limits, error, scope, and internal evidence index.
- Add deterministic node-to-file and node-to-community indexes with explicit
  duplicate/missing diagnostics.
- Move source-scope classification out of `callflow_model.rs` into the new
  module and retain classification reason.
- Deepen generated-source evidence in `compass-files`/`compass-core`, including
  bounded admitted VCS metadata and negative fixtures.
- Build independent Production and All-code node realizations.
- Run the new projection side-by-side in tests without publishing it publicly.

Primary files:

- new `crates/compass-output/src/architecture_projection/{mod.rs,scope.rs,index.rs,error.rs}`
- `crates/compass-output/src/lib.rs`
- generated evidence owner in `crates/compass-files`
- `crates/compass-core/src/pipeline.rs`
- `crates/compass-output/tests/architecture_projection_fixtures.rs`

Acceptance:

- Generated and Vendor leakage into Production is zero for every fixture.
- All-code retains every node and exact counts for every source scope.
- Missing source remains Unknown.
- The tracked generated-bundle reproduction no longer affects Production
  membership or candidate names.
- Classification is deterministic across path separator and input-order
  permutations.

Suggested commit sequence:

1. Deepen generated-file evidence with positive and negative fixtures.
2. Add the private projection module, bounded indexes, and error model.
3. Move source classification behind the seam and build both scope realizations.

Verification:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo test -p compass-files -p compass-core -p compass-output --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo clippy -p compass-files -p compass-core -p compass-output \
  --all-targets --all-features --locked -- -D warnings
```

Rollback: the side-by-side module can be removed without changing the existing
export. Generated-evidence changes remain only if their independent graph
contract tests pass.

### Phase 2: Classify relationships and make counts honest

Purpose: give architecture routes one semantic owner and stop presenting every
edge as a call.

Deliverables:

- Add the closed architecture relation policy and complete relation table.
- Classify every graph relationship into execution, dependency, type,
  structure, contextual, or unknown.
- Build per-scope internal, cross-group, and unassigned coverage by class.
- Preserve original relations and evidence on every detail relationship.
- Add deterministic route aggregation and relation-mix summaries.
- Add quality diagnostics for unknown and unassigned relationships.

Primary files:

- new `crates/compass-output/src/architecture_projection/relation.rs`
- new `crates/compass-output/src/architecture_projection/routes.rs`
- `crates/compass-output/tests/architecture_projection_fixtures.rs`
- public relation reference tests in the lowest owning crate

Acceptance:

- `contains` and `references` do not contribute to default Architecture route
  counts.
- Execution lens exact-call counts equal exact admitted `calls` edges.
- For every scope/lens, internal + cross-group + unassigned equals admitted.
- Unknown relation names are retained, bounded, and disclosed.
- Edge direction, multiplicity, confidence, and provenance are unchanged.

Suggested commit sequence:

1. Freeze and implement the complete relation-class table.
2. Add per-scope/lens coverage and route aggregation.
3. Add unknown/unassigned diagnostics and invariant tests.

Verification:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo test -p compass-output --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo test -p compass-model --locked
```

Rollback: revert the relation-policy slice; source projection remains useful
and unexposed until the coordinated cutover.

### Phase 3: Add hierarchy, ranking, naming, identity, and quality

Purpose: complete the internal architecture model and eliminate the need for
flat top-N plus `Other`.

Deliverables:

- Resolve source-backed owners and leaf community groups.
- Reuse `compass-graph` cohesion, connectivity, stable remapping, and membership
  signatures rather than duplicating clustering.
- Implement the ranked overview and exact omission records.
- Implement bounded multi-signal naming and duplicate/generic/minified rejection.
- Separate stable IDs from names and persist naming provenance/signatures.
- Implement quality metrics, diagnostics, and calibrated status thresholds.
- Prove no route references an omission summary.

Primary files:

- new `crates/compass-output/src/architecture_projection/{hierarchy.rs,rank.rs,names.rs,quality.rs,model.rs}`
- `crates/compass-graph/src/cluster.rs` only for reusable topology helpers that
  are genuinely owned there
- `crates/compass-core/src/cluster_existing.rs`
- `crates/compass-semantic/src/community_labels.rs`
- architecture projection tests and qualification fixtures

Acceptance:

- No automatic group is named `Other`.
- Every omitted group is reachable by stable ID through directory/search data.
- Duplicate display names are zero after evidence-based disambiguation.
- The Compass fixture produces multiple project-specific owner/subsystem names
  and no repeated `Crates Compass Output` labels.
- Provider-free runs are byte deterministic.
- Provider labels with stale membership signatures are rejected.
- Quality status matches the named calibrated gates and includes exact witnesses.

Suggested commit sequence:

1. Add owner hierarchy, stable IDs, ranking, and omission records.
2. Add deterministic candidate naming and evidence-based quality gates.
3. Integrate signature-valid persisted/provider labels.
4. Calibrate and freeze quality status thresholds on the Phase 0 corpora.

Verification:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo test -p compass-graph -p compass-output -p compass-core \
  -p compass-semantic --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  ./scripts/qualify_code_graph_v1.sh --fixtures-only
```

Rollback: ranking, naming, and quality commits remain separate behind the
unpublished module. Revert a policy slice rather than adding caller-side
exceptions.

### Phase 4: Publish the architecture contract and cut over consumers

Purpose: ship the correct semantic model atomically across Rust, CLI,
workbench, VS Code, and the shared viewer.

Deliverables:

- Publish strict Rust serialization and validation for
  `compass.viewer.architecture/1`.
- Update `callflow-json`, `callflow-html`, capability reporting, workbench view
  embedding, and CLI contract tests.
- Add matching TypeScript runtime schemas and malicious/oversized fixture tests.
- Replace TypeScript `callflowOverview` semantic derivation with filtering and
  aggregation over Rust-classified scopes, groups, and relations.
- Update the VS Code retained architecture index and paged message contracts.
- Coordinate minimum CLI compatibility and unknown-major rejection.
- Update reference docs, `COMPATIBILITY.md` if required, `MIGRATION.md`, and
  `CHANGELOG.md`.
- Rebuild and verify generated viewer assets from `packages/compass-viewer`.

Primary files:

- `crates/compass-output/src/{lib.rs,callflow.rs,callflow_model.rs,workbench.rs}`
- `crates/compass-cli/src/{lib.rs,capability_commands.rs,help.rs}`
- `crates/compass-cli/tests/viewer_export_cli.rs`
- `packages/compass-viewer/src/contracts/`
- `packages/compass-viewer/src/workbench/VisualizationWorkbench.tsx`
- `editors/vscode/src/views/{architectureIndex.ts,architecturePanel.ts}`
- `editors/vscode/src/transport/architectureMessages.ts`
- compatibility and reference documents

Acceptance:

- All consumers accept only the new known architecture major.
- Production grouping/naming is identical in standalone HTML and VS Code.
- All-code restores all scoped nodes without regrouping Production in the UI.
- User-visible counts say relationships except for exact-call views.
- No full model crosses the VS Code host/webview seam.
- Existing 8 MiB/128 MiB process limits and bounded page sizes remain unchanged.
- Old direct JSON consumers receive documented upgrade guidance rather than a
  silently reinterpreted v1 payload.

Suggested commit sequence:

1. Publish and validate the Rust architecture contract and CLI serialization.
2. Add TypeScript runtime schemas and update capability compatibility.
3. Cut over VS Code indexing/messages and shared workbench consumption.
4. Update docs, migration/release notes, and generated viewer assets.

Verification:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo test -p compass-output -p compass-cli --locked
npm run typecheck:js
npm run test:js
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
sh scripts/check_product_boundary.sh
```

Rollback: the contract cutover is one coordinated release slice. Revert the
whole slice before release; do not emit v1 with v2 semantics.

### Phase 5: Deliver hierarchical interaction and quality UX

Purpose: make the new model legible and actionable rather than merely correct.

Deliverables:

- Render owner overview and leaf-group drill-down with deterministic layout.
- Remove all special layout treatment for an `other` ID.
- Add Architecture, Execution, Dependency, Type, and Structure lenses.
- Show separate Extraction and Architecture quality statuses.
- Add omission disclosure, directory navigation, name provenance, and quality
  diagnostic actions.
- Update inspector terminology and accessible table alternatives.
- Capture approved wide, narrow, light, dark, and high-contrast screenshots.

Primary files:

- `packages/compass-viewer/src/architecture/`
- `packages/compass-viewer/src/workbench/VisualizationWorkbench.tsx`
- `packages/compass-viewer/src/theme.css`
- `editors/vscode/src/webviews/architecture.tsx`
- `tests/viewer/`
- `docs/assets/screenshots/`

Acceptance:

- No rendered node or route uses `Other` as an omission device.
- The initial Compass view is project-specific and not dominated by generated
  viewer assets.
- Every shown metric has a clear denominator or scope.
- Keyboard, reduced-motion, high-contrast, and narrow layouts expose the same
  information.
- Search and directory navigation reach a group omitted from the map.
- Screenshot review shows no repeated generic labels and no false mega-hub.

Suggested commit sequence:

1. Render owner/leaf navigation and remove `other` layout behavior.
2. Add relation lenses, omission disclosure, and architecture-quality status.
3. Complete inspector, accessible table, responsive, and screenshot evidence.

Verification:

```bash
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
# Run the focused Chromium viewer suite under tests/viewer.
```

Rollback: React presentation commits are independently revertible while the
new contract remains inspectable through JSON and tables.

### Phase 6: Add project overlay and real-repository qualification

Purpose: let projects own domain language where automatic evidence is
insufficient and prove the default remains robust without configuration.

Deliverables:

- Define and validate the first architecture overlay schema.
- Adapt existing `--sections` input with explicit deprecation/migration rules.
- Add optional `.compass` configuration and documented precedence.
- Add Cargo workspace, npm monorepo, mixed-language, generated-heavy, vendor-
  heavy, test-heavy, single-package, and sparse-relationship qualification
  corpora.
- Measure quality status, stability, runtime, peak RSS, and output size.
- Publish before/after screenshots and exact qualification evidence.

Primary files:

- overlay model/loader in the lowest owning Compass crate
- `compass-output::architecture_projection`
- CLI parsing/help/tests
- `docs/reference/configuration.md`
- `docs/reference/commands.md`
- `docs/reference/outputs.md`
- qualification scripts and reviewable fixture expectations

Acceptance:

- Zero-configuration fixtures meet the calibrated quality gates.
- An overlay can improve vocabulary without changing graph edges or identities.
- Invalid/overlapping overlays fail with source-located diagnostics.
- Equivalent repeated runs are byte deterministic.
- Small edits preserve unaffected group IDs and report bounded identity/name
  churn.
- Runtime, memory, and payload remain within thresholds recorded before this
  phase; no performance claim is published without those measurements.

Suggested commit sequence:

1. Add the strict overlay schema, selectors, and validation.
2. Add CLI/project configuration adapters and migration documentation.
3. Add external corpora, performance replay, and published qualification evidence.

Verification:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo fmt --all -- --check
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo clippy --workspace --lib --bins --locked -- -D warnings
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  cargo test --workspace --lib --bins --locked
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-3058 \
  ./scripts/qualify_code_graph_v1.sh --fixtures-only
sh scripts/check_product_boundary.sh
```

Rollback: overlay support is optional and can be reverted independently. Do not
relax automatic quality gates to make one overlay fixture pass.

## Cross-phase test matrix

| Risk | Lowest test seam | Contract/integration evidence |
| --- | --- | --- |
| generated leakage | source classifier and graph file inventory | Compass generated viewer bundle fixture |
| path portability | normalized source-scope tests | Windows separator export fixture |
| relation mislabeling | relation policy table | viewer count/terminology test |
| direction/multiplicity loss | route aggregation unit tests | CLI JSON round-trip |
| false `Other` subsystem | hierarchy/omission tests | browser screenshot and route inspection |
| repeated/generic names | name candidate/gate tests | Compass real-repository snapshot |
| stale provider label | membership-signature tests | cluster-existing round trip |
| unstable IDs/order | permutation and edit tests | repeated byte hashes |
| hidden data | omission sum invariants | search/drill-down browser test |
| misleading quality | quality formula tests | header/status accessibility test |
| oversized model | projection limit tests | 128 MiB host ceiling and bounded pages |
| unsafe overlay | parser/selector negative tests | CLI no-partial-output test |

## Performance budgets

Phase 0 records the baseline before numeric budgets are frozen. The initial
design targets are:

- projection work linear in nodes plus relationships, excluding bounded sorting
  of group/route summaries;
- no second complete clone of node or relationship records;
- no renderer-side full-graph semantic recomputation;
- bounded candidate naming per group;
- overview groups and routes capped independently;
- standalone export and VS Code retained model below the existing 128 MiB
  architecture ceiling for the existing Django qualification corpus; and
- no unexplained regression greater than 10% in median projection time or peak
  RSS on the pinned corpora.

These are qualification gates, not published product claims. A failure pauses
the phase for diagnosis; it does not justify raising a limit silently.

## Security and privacy

- Treat labels, paths, overlays, and graph attributes as untrusted text at every
  render seam.
- Keep overlay paths project-contained and normalize separators portably.
- Never execute overlay content or construct shell commands.
- Bound file reads, candidate strings, diagnostics, and serialized output.
- Do not log source-derived labels, paths, or symbols from the VS Code host
  beyond existing user-visible diagnostics.
- Optional provider naming remains explicitly configured and receives only the
  existing bounded community summary; architecture projection itself is local.
- Reuse atomic write and validation primitives for every output.

## Documentation updates at completion

- `docs/reference/outputs.md`: architecture schema, scopes, lenses, omissions,
  quality, and naming provenance.
- `docs/reference/commands.md`: call-flow export behavior and overlay options.
- `docs/reference/configuration.md`: optional overlay and precedence.
- `docs/cookbook/architecture-discovery.md`: overview-to-drill-down workflow.
- `COMPATIBILITY.md`: coordinated schema/capability effect when applicable.
- `MIGRATION.md`: direct `compass.viewer.callflow/1` consumer migration.
- `CHANGELOG.md`: release-visible architecture correction.

Implementation documents describe planned behavior, not shipped evidence.
Reference and cookbook claims must change only in the phase that ships the
corresponding contract.

## Global definition of done

The architecture hardening is complete when:

- Production grouping and naming cannot observe Generated, Vendor, Test,
  Documentation, or Unknown nodes;
- every architecture relationship has one deterministic class while preserving
  original graph evidence;
- no count called `calls` includes non-call relationships;
- no automatic `Other` node exists;
- omitted groups and routes have exact bounded disclosure and remain reachable;
- names are unique, project-specific, source-grounded, signature-protected, and
  independent from identity;
- architecture quality is distinct from extraction completion;
- HTML, workbench JSON, and VS Code use the same Rust-owned projection;
- the new machine contract, migration, and capability behavior are documented
  and tested;
- the Compass screenshot regression and representative external corpora pass
  the declared quality gates; and
- native Rust, JavaScript, viewer asset, product-boundary, and code-graph
  qualification gates pass with no unrelated worktree changes.

## Delivery record

Phases 0 through 6 were delivered in order and accepted against Compass's own
150,127-node graph. Exact metrics, screenshots, gate results, and the one
unrelated residual test failure are recorded in the
[architecture graph hardening qualification](architecture-graph-hardening-qualification.md).
