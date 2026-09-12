# Community detection quality technical design

Status: implemented; automatic selector remains qualification-only

Scope: `compass-graph` community topology, detection, quality, incremental
updates, and `compass-core` orchestration

Implementation status (2026-09-12): phases 0 through 7 are implemented. The
production profile uses fixed-resolution typed Leiden. The bounded
three-candidate selector remains qualification-only because its compact
performance measurement did not justify enabling it by default. See the
linked qualification report.

Extends:
[Architecture graph hardening technical design](architecture-graph-hardening-phased-technical-design.md)

## Overview

Compass currently derives graph communities with a deterministic native
multi-level Louvain implementation. It uses weighted edges, a fixed seed,
optional hub exclusion, stable remapping, oversized-community splitting, and
an internal-density score called cohesion.

The implementation is fast and reproducible, but four limitations prevent
Compass from demonstrating that a changed partition is better:

1. clustering collapses most typed Base Graph relationships into equivalent
   undirected edges;
2. Louvain can return weakly connected or disconnected communities;
3. one fixed resolution is accepted before a small set of hard-coded split
   rules runs; and
4. cohesion measures only internal density, not separation, connectedness,
   stability, or agreement with reviewed expectations.

This design introduces one native community module with a small public
interface and five internal responsibilities:

```text
validated Base Graph
        |
        v
versioned community topology
        |
        +---------------------> quality evaluator
        |                              ^
        v                              |
bounded partition candidates ---------+
        |
        v
deterministic selector
        |
        v
stable IDs + labels + quality evidence
```

The implementation path deliberately separates measurement, structural
refactoring, topology semantics, detector changes, and rollout. No behavior
change is accepted only because it increases modularity or produces more
communities.

## Relationship to architecture projection

This design changes graph-derived community detection. It does not replace the
existing `compass-output::architecture_projection` module.

The two quality domains remain distinct:

| Domain | Owner | Meaning |
| --- | --- | --- |
| Community quality | `compass-graph` | Connectivity and separation of one graph partition |
| Architecture quality | `compass-output::architecture_projection` | Source-scope correctness, grouping, naming, coverage, omissions, and presentation quality |

The Architecture projection continues to classify Production, Test,
Generated, Vendor, Documentation, and Unknown before architecture grouping.
It may consume community-quality evidence, but it must not redefine the
detector or mutate the Base Graph.

Communities remain graph-derived hypotheses. They are not official modules or
hand-maintained architecture declarations.

## Current implementation

The current path is distributed across several callers:

```text
compass-core::pipeline
  -> convert typed graph to legacy node-link projection
  -> cluster_incremental
       -> WeightedGraph::from_document
       -> seeded Louvain
       -> hub reattachment
       -> large/low-density split passes
       -> stable ID remapping
  -> label_communities_by_hub
  -> score_communities
  -> graph insights and output
```

`cluster-only`, historical normalization, architecture projection, and viewer
model construction repeat parts of this sequence.

### Current topology semantics

`WeightedGraph::from_document`:

- sorts node IDs;
- ignores dangling endpoints;
- excludes only table-navigation containment;
- reads `weight`, defaulting to `1.0`;
- collapses duplicate directed endpoint pairs;
- projects the selected edges into symmetric adjacency; and
- does not otherwise use relationship kind, direction, multiplicity,
  confidence, or provenance.

### Current quality semantics

The public cohesion value is:

```text
unique internal endpoint pairs / (member_count * (member_count - 1) / 2)
```

Singletons receive `1.0`. External cut edges do not affect the score. The
detector uses a separate topology-derived density calculation for its split
rule, while analysis has another cohesion implementation. These calculations
can observe different effective graphs.

### Current incremental semantics

An incremental update admits changed nodes, their previous communities, and
the immediate community boundary. It clusters the induced affected subgraph
while freezing all other assignments. Edges from the affected region to
frozen communities are absent from the local objective. The implementation
falls back to a full run when the admitted region exceeds 4,096 nodes or 25%
of the current graph.

## Goals

- Make detector quality measurable independently from architecture rendering.
- Give detection and quality exactly the same versioned topology.
- Use relationship kind, evidence confidence, multiplicity, and direction
  through an explicit deterministic policy.
- Guarantee that every non-isolate community is internally connected.
- Improve small-module recovery without allowing unbounded resolution search.
- Preserve deterministic output for equivalent graph and profile inputs.
- Keep structural community detection native, local, bounded, and credential
  free.
- Preserve Base Graph node, relationship, direction, multiplicity, identity,
  and provenance contracts.
- Keep previous community numbering out of content-addressed historical
  realization identity.
- Detect incremental quality degradation and fall back to a full run.
- Publish inspectable quality evidence without pretending that one partition
  is the only correct architecture.
- Establish real-repository and planted-partition qualification before changing
  the default detector.

## Non-goals

- Add Graphify or another runtime, test, configuration, or fallback dependency.
- Call a provider, model, embedding service, or vector database.
- Add a dynamic clustering plugin system.
- Infer business ownership from display labels or path-name similarity.
- Rewrite the Base Graph to fit a preferred community partition.
- Hide Test, Generated, Vendor, Documentation, or Unknown records from the Base
  Graph.
- Make a prior working-tree partition influence an exact historical
  realization.
- Treat higher modularity, lower conductance, or package agreement alone as
  ground truth.
- Replace architecture overlays or explicit owner groups with detected
  communities.
- Introduce overlapping community membership in this implementation line.
- Adopt Constant Potts Model semantics under the existing `--resolution`
  option. CPM may be evaluated later under a separately versioned profile.

## Required invariants

### Base Graph fidelity

- Community topology is a derived read view; Base Graph records are unchanged.
- Every admitted topology edge retains counts by relationship kind and
  confidence in its topology evidence.
- Direction and multiplicity may be projected for detection only by an
  explicit versioned rule.
- Ambiguous or unsupported meaning never becomes an exact relationship.
- Unknown relationship kinds fail profile validation; they are not silently
  assigned a default semantic weight.

### Determinism

- Equivalent Base Graphs and community profiles produce equivalent memberships,
  quality evidence, diagnostics, and ordering.
- Node and edge input order cannot change the result.
- Node visitation uses a fixed recorded seed and canonical starting order.
- Candidate ties resolve by a canonical partition digest, never hash-map or
  filesystem iteration.
- Floating-point comparisons use one named tolerance and deterministic
  secondary ordering.
- Parallel evaluation collects candidates into canonical order before
  selection.

### Bounded work

- Topology nodes, input relationships, projected pairs, total weight,
  candidates, levels, local moves, quality visits, and incremental affected
  nodes all have explicit limits.
- Candidate generation uses a fixed schedule with at most three partitions in
  the first production version.
- Exhausting a limit returns a typed error with required and allowed work. It
  never returns an empty or partial partition as success.
- Qualification may run a wider offline matrix, but normal builds remain under
  the production limits.

### Partition integrity

- Every admitted node belongs to exactly one community.
- Every member ID exists in the input graph.
- Every non-isolate community is internally connected in the selected topology.
- Community member lists and community ordering are canonical.
- Stable remapping changes IDs only; it cannot change membership or quality.
- A failed postcondition prevents publication of the new coherent artifact set.

### History

- Algorithm, topology, quality, selector, seed, resolution policy, hub policy,
  and all meaning-affecting limits enter the historical build profile.
- Existing historical realizations remain immutable and queryable.
- A rebuild uses the running engine profile and creates a new realization; it
  does not rewrite the old one.
- Operational reuse of previous community IDs remains excluded from historical
  content identity.

## Design decisions

### 1. Qualify before changing the objective

The first phase adds measurements and fixtures without modifying production
memberships. It records both strengths and known weaknesses of
`seeded-louvain/v1`.

The qualification suite contains three evidence classes:

1. compact hand-reviewed topology fixtures;
2. deterministic planted-partition graphs with known memberships; and
3. pinned public repositories with reviewable subsystem expectations.

Package or directory agreement is supporting evidence, not truth. A codebase
can intentionally place one subsystem across several packages or several
subsystems in one package.

### 2. Deepen one community module

The current free-function family becomes one module owned by `compass-graph`.
Its public interface accepts a validated graph plus one complete request and
returns one complete result.

Target ownership map:

```text
crates/compass-graph/src/community/
|-- mod.rs          public facade, validation, complete result
|-- topology.rs     versioned Base Graph projection
|-- quality.rs      partition and per-community evidence
|-- detection.rs    Louvain compatibility and native Leiden
|-- selection.rs    bounded candidate schedule and ordering
|-- incremental.rs  affected region, frozen influence, fallback
`-- labels.rs       stable remapping, signatures, base labels
```

This is an ownership map, not a requirement to maximize file count. Files stay
combined when their invariants cannot be understood or tested independently.

### 3. Preserve a behavior-equivalent compatibility profile

Before adding typed weighting, `topology.rs` reproduces the current
`WeightedGraph::from_document` behavior. The Louvain implementation moves
behind the new facade without semantic edits.

The compatibility profile remains identified as:

```text
algorithm: seeded-louvain/v1
topology: legacy-undirected/v1
quality: density/v1
selector: fixed-resolution/v1
seed: 42
```

Cold, warm, input-permuted, cluster-only, and historical fixture results must
remain byte-equivalent before later phases begin.

### 4. Add a typed evidence topology

The new topology consumes typed `compass.graph/1` relationships. A legacy
node-link adapter remains only where direct reclustering compatibility requires
it. Both adapters feed the same validated internal topology.

The first typed topology remains undirected for partition detection, but its
symmetrization is explicit:

```text
pair strength = bounded forward evidence + bounded reverse evidence
```

Reciprocal relationships therefore contribute more evidence than a one-way
relationship without erasing their original directions. The topology result
retains forward, reverse, relation, confidence, and omission summaries for
inspection.

Relationship kinds are assigned exhaustively to closed strength classes:

| Strength class | Candidate relationship families | Intent |
| --- | --- | --- |
| Strong | calls, handles, routes, reads/writes, messaging, scheduling | Runtime or domain flow |
| Medium | imports, depends-on, instantiates, registers, inheritance | Dependency and type coupling |
| Weak | containment, references, exports, aliases, tests, documents | Structural or navigational support |
| Excluded | invalid or profile-inapplicable evidence | Retained in Base Graph, absent from this topology |

The exact `EdgeKind` table and integer weights are frozen only after Phase 1
qualification. The table is compile-time exhaustive, reviewed in one file,
and versioned as part of the topology identity.

Evidence confidence modifies, but never upgrades, relationship strength:

| Evidence | Initial candidate policy |
| --- | --- |
| Exact/source-derived | Full relation strength |
| Inferred with complete provenance | Reduced relation strength |
| Ambiguous | Excluded from selection topology and counted in evidence |

Repeated evidence uses a bounded saturating contribution rather than either
discarding all multiplicity or allowing generated repetition to dominate. The
initial cap is four distinct source-anchored occurrences per unordered node
pair and relationship kind. Qualification may lower the cap before the policy
is frozen.

The graph's finite positive `weight` remains an input factor. Normalization and
the maximum aggregate pair weight are explicit profile constants. Invalid or
overflowing values fail topology construction rather than becoming zero.

Source scope does not enter the first typed community topology. Production
source isolation remains owned by Architecture projection. A future
scope-specific partition would require a separate profile and contract rather
than importing output-owned path rules into `compass-graph`.

### 5. Implement native Leiden with modularity

The first detector improvement keeps the current generalized modularity
objective and resolution meaning. It adds the Leiden refinement phase rather
than changing both algorithm and objective together.

Each level performs:

1. deterministic seeded local moving;
2. refinement inside each coarse community from a connected singleton
   partition;
3. aggregation using the refined partition; and
4. another level until improvement stops or the level limit is reached.

Refinement permits only moves that preserve the connectedness conditions of
the candidate community. The selected result is validated independently with
a bounded connected-components pass. A validation failure is a detector error,
not a silent repair.

The implementation remains native Rust inside `compass-graph`; it adds no
external runtime or clustering library. Small checked-in oracle fixtures may
be derived once from the published algorithm description, but normal tests do
not execute Python, Java, or a network service.

The detector identity becomes:

```text
seeded-leiden-modularity/v1
```

The Constant Potts Model is deliberately deferred. Its resolution parameter
has a different density interpretation, so reusing the current option would be
an incompatible semantic shortcut.

### 6. Select from a bounded resolution schedule

When the user does not explicitly supply `--resolution`, the quality profile
may generate at most three candidates around the configured base resolution:

```text
3/4 * base, base, 4/3 * base
```

Rational multipliers avoid a hidden arbitrary sweep. Each resulting partition
is evaluated at the common base resolution so values are comparable. An
explicit `--resolution N` requests one fixed candidate at exactly `N` and
therefore preserves direct user control.

Selection is lexicographic and inspectable, not an opaque weighted sum:

1. reject incomplete or invalid candidates;
2. reject candidates with disconnected non-isolate communities;
3. retain candidates within the named modularity tolerance of the best
   base-resolution modularity;
4. minimize communities that violate the qualified size and separation
   thresholds;
5. minimize weighted boundary conductance;
6. minimize non-isolate singleton fragmentation; and
7. break remaining ties by canonical partition digest.

Candidate schedules, rejected candidates, metrics, and the final selection
reason are part of quality evidence.

The current oversized and low-density one-shot split rules remain in the
compatibility profile. They are removed from the new profile once Leiden and
candidate selection satisfy the same pathological-size fixtures. A refinement
pass must never silently reset a user-supplied resolution to `1.0`.

### 7. Evaluate a quality vector, not one magic score

`CommunityQuality` records evidence rather than claiming universal correctness.

Per-community evidence includes:

- member count;
- isolate status;
- connected-component count;
- internal unique edge count and internal weight;
- boundary edge count and boundary weight;
- volume;
- density;
- conductance;
- modularity contribution;
- relation-strength mix;
- evidence-confidence mix; and
- bounded witness node and edge IDs for degraded conditions.

Partition evidence includes:

- assigned and omitted node counts;
- community and non-isolate singleton counts;
- disconnected-community count;
- total modularity at the base resolution;
- weighted mean and worst retained conductance;
- largest-community fraction;
- candidate agreement and selection reason;
- algorithm/topology/quality/selector identities; and
- exact limits and omissions.

Density remains available as the compatibility `cohesion` projection. It is
not used alone to label a partition good. `CommunityMetadata.score` remains
unset until Compass defines and versions a scalar meaning; the quality vector
must not be compressed into that existing optional field prematurely.

### 8. Preserve frozen influence during incremental updates

The incremental topology represents each adjacent frozen community as one
locked anchor node. Edge weights from affected nodes to that community are
aggregated onto the anchor.

```text
frozen community A ----\
                        affected topology ---- frozen community B
frozen community C ----/
```

Affected nodes may join or leave an anchored community. Locked anchors cannot
move, and two different frozen anchors cannot merge in a local run. A local
community without an anchor receives a deterministic new ID after selection.

After local clustering, the complete merged partition is evaluated against the
complete current topology. The implementation falls back to a full run when:

- any partition invariant fails;
- a non-isolate community is disconnected;
- base-resolution modularity is worse than the deterministic prior-plus-new-
  singleton baseline beyond tolerance;
- dominant-community or conductance limits are newly violated;
- two frozen anchors would merge;
- affected work exceeds the existing absolute or fractional limit; or
- a quality or topology limit is exhausted.

Repeated edit, revert, rename, and delete sequences compare the final
membership with a clean full build. Exact membership equality is required for
the deterministic fixtures where the optimum is unique; quality equivalence
and explicit stable-ID rules apply where multiple partitions tie.

Temporal agreement with previous working-tree assignments is diagnostic only.
It cannot influence an exact historical realization.

## Proposed Rust interface

Names are illustrative but the ownership and information flow are required.

```rust
pub struct CommunityRequest<'a> {
    pub profile: CommunityProfile,
    pub resolution: ResolutionPolicy,
    pub exclude_hubs_percentile: Option<f64>,
    pub previous: Option<&'a PreviousCommunities>,
    pub changed_sources: &'a BTreeSet<String>,
    pub limits: CommunityLimits,
}

pub enum ResolutionPolicy {
    Auto { base: f64 },
    Fixed(f64),
}

pub enum CommunityProfile {
    CompatibilityV1,
    QualityV1,
}

pub struct CommunityResult {
    pub communities: Communities,
    pub base_labels: BTreeMap<usize, String>,
    pub signatures: BTreeMap<usize, String>,
    pub quality: PartitionQuality,
    pub execution: CommunityExecution,
    pub identity: CommunityIdentity,
}

pub enum CommunityExecution {
    Full,
    Incremental { affected_nodes: usize },
    FullFallback { affected_nodes: usize, reason: FallbackReason },
}

pub fn build_communities(
    document: &compass_model::code_graph::GraphDocument,
    request: &CommunityRequest<'_>,
) -> Result<CommunityResult, CommunityError>;
```

The existing `cluster`, `cluster_incremental`, `score_communities`, stable
remapping, signature, and hub-label functions remain temporarily as
compatibility facades. New production callers use `build_communities`; the
facades are removed or made crate-private only after downstream migration.

Two input adapters justify the graph seam during migration:

- the typed Base Graph adapter for normal builds and history; and
- the validated legacy node-link adapter for supported direct reclustering.

Both adapters must produce the same internal topology when their source facts
are equivalent.

## Error and limit model

Community work returns typed errors:

```text
invalid_profile
invalid_resolution
invalid_edge_weight
unknown_relationship_kind
dangling_endpoint
topology_limit_exceeded
candidate_limit_exceeded
move_limit_exceeded
quality_limit_exceeded
partition_incomplete
partition_disconnected
partition_duplicate_member
partition_unknown_member
```

Each limit error reports:

```text
stage
required
limit
processed
```

The normal pipeline treats these as build failures and retains the prior
coherent artifact set. `cluster-only` writes no partially reclustered graph.
Qualification tools may record a failed candidate, but the production selector
cannot select one.

The initial production limits retain the current 10-level ceiling and
incremental 4,096-node/25% ceilings. Phase 1 measurements establish explicit
move, projected-pair, weight, and quality-visit ceilings before the new profile
is enabled.

## Quality qualification

### Fixture families

The native fixture suite includes:

- two dense groups joined by one bridge;
- a ring of cliques that exposes modularity resolution behavior;
- an articulation graph that can create a disconnected Louvain community;
- stars and multi-hub graphs;
- directed fan-in and fan-out graphs;
- reciprocal versus one-way relationships;
- parallel exact, inferred, and ambiguous evidence;
- containment-dominated file graphs;
- isolated nodes mixed with connected subsystems;
- one very large weak community;
- layered handler/domain/repository code topology;
- tests and documentation connected to production code; and
- input-order and edge-order permutations of every compact case.

Deterministic LFR-style fixtures cover heterogeneous degree and community-size
distributions at several mixing levels. The generator or generated fixture is
native and pinned; qualification never downloads a corpus.

### Metrics

Where planted membership exists, qualification records:

- adjusted Rand index;
- adjusted mutual information;
- exact recovery rate; and
- false merge/split counts.

For all graphs it records:

- connectedness;
- modularity on a named common topology and resolution;
- per-community and weighted conductance;
- density;
- largest-community and non-isolate singleton fractions;
- deterministic repeat and permutation equality;
- edit/revert stability;
- clustering wall time; and
- peak resident memory.

Metrics from different topology identities are never compared as if they used
the same denominator.

### Real repositories

Release qualification uses pinned public repositories already present under
`/Volumes/Workspace/Github` where available. Existing checkouts are read-only.
Missing authorized public corpora are cloned only under that mounted volume.

The corpus should cover:

- a Rust workspace;
- a Java or Kotlin multi-module project;
- a TypeScript monorepo;
- a Python package with tests and documentation;
- a Go multi-package project;
- a frontend/backend project; and
- Compass itself.

Each corpus records commit, build profile, topology identity, reviewed
subsystem expectations, and exact omissions. Repository paths or package names
may support review but cannot be the sole accuracy oracle.

### Acceptance gates

The new default is eligible only when all of these hold:

- zero disconnected non-isolate communities across fixture and pinned-corpus
  runs;
- exact deterministic repeat and input-permutation equality;
- exact recovery for unambiguous planted fixtures;
- no planted-fixture adjusted metric regression greater than 0.02 and a net
  improvement on resolution-limit and articulation families;
- no unexplained real-repository dominant-community or singleton regression;
- no Base Graph node, edge, identity, direction, multiplicity, or provenance
  change;
- cold full-build wall time no more than 15% above compatibility on the pinned
  median;
- clustering-stage wall time no more than 50% above compatibility on the
  pinned median;
- peak RSS no more than 10% above compatibility;
- incremental one-file updates stay within their documented node and memory
  bounds; and
- every regression includes bounded witness evidence.

If the three-candidate auto schedule misses the performance gates, the default
remains fixed-resolution Leiden and multi-resolution selection stays
qualification-only until optimized.

## Compatibility and versioning

The default detector change is compatibility-sensitive even if
`compass.graph/1` remains unchanged, because community membership affects
navigation, reports, architecture grouping, history comparisons, and derived
artifacts.

Meaning-affecting identities move from CLI literals into `compass-graph` and
enter every current and historical build profile:

```text
cluster_algorithm
cluster_topology
cluster_quality
cluster_selector
cluster_seed
cluster_resolution_policy
cluster_hub_policy
cluster_limits_version
```

The first qualified target identities are:

```text
cluster_algorithm = seeded-leiden-modularity/v1
cluster_topology = typed-evidence-undirected/v1
cluster_quality = community-quality/v1
cluster_selector = bounded-multiresolution/v1
```

If auto selection does not qualify, the selector identity is instead
`fixed-resolution/v1`.

The configuration digest changes whenever a meaning-affecting identity or
option changes. Existing output is rebuilt coherently rather than partially
reusing old memberships.

`--resolution N` remains supported and means fixed generalized-modularity
resolution `N`. Omitting it may become bounded automatic selection after that
behavior qualifies. Help and reference documentation must state the
difference.

`--exclude-hubs N` remains explicit. Hub removal and reattachment operate on
the selected topology and are included in quality evidence. A hub cannot be
reattached solely by an ambiguous excluded edge.

The rollout requires:

- native CLI and graph regression coverage;
- command and output reference updates;
- a `MIGRATION.md` note explaining expected community-ID and membership churn;
- a `CHANGELOG.md` entry;
- updated history-profile fixtures; and
- a new qualification report with exact corpus identities and measurements.

Published historical realizations are never rewritten. No compatibility mode
silently substitutes the old detector for the new profile.

## Quality evidence publication

Phases 1 through 5 keep the richer quality result internal and in qualification
reports while its meaning stabilizes. Existing `cohesion` output remains the
compatibility density projection.

After detector acceptance, normal clustered builds may add the strict artifact:

```text
community-quality.json
schema: compass.community-quality/1
```

The artifact contains:

- graph generation and canonical graph digest;
- complete community profile identity;
- selected and rejected candidate summaries;
- per-community quality evidence;
- partition evidence;
- diagnostics with bounded witnesses;
- limits and exact omissions; and
- a canonical result digest.

It joins the guarded coherent artifact set and immutable history realization.
Unknown majors fail explicitly. A missing artifact on an older graph means
quality evidence is unavailable, not zero or good.

The human report may summarize this artifact. Renderers cannot recalculate or
reinterpret its status. Architecture projection may reference its community
metrics only after validating graph generation, digest, and profile identity.

## Implementation phases

### Phase 0: approve design and freeze current behavior

Actions:

1. Add current Louvain, topology, split, remapping, and density behavior tables.
2. Add byte-equivalence fixtures for normal, cluster-only, and historical paths.
3. Record current clustering stage time, RSS, memberships, modularity, density,
   connectedness, conductance, and size distribution.
4. Record the existing algorithm identity in one `compass-graph` constant.

Done when:

- production behavior is unchanged;
- every current split threshold has a named characterization test; and
- the baseline report is reproducible.

Suggested commits:

1. `test(graph): characterize community topology and quality`
2. `docs(graph): record community detection baseline`
3. `refactor(graph): centralize clustering identity constants`

### Phase 1: build the quality qualification seam

Actions:

1. Implement shared internal topology statistics.
2. Implement per-community and partition quality evidence.
3. Make current cohesion call the shared density calculation.
4. Add planted and pathological fixtures.
5. Add a focused native qualification runner and deterministic JSON report.
6. Capture the pinned real-repository baseline.

Done when:

- all quality consumers observe the same compatibility topology;
- formulas have direct reference tests;
- every metric reports its denominator and omissions; and
- no production membership changes.

Suggested commits:

1. `feat(graph): add shared community quality evidence`
2. `test(graph): add planted community qualification fixtures`
3. `test(graph): add deterministic community quality runner`
4. `docs(graph): publish compatibility quality baseline`

### Phase 2: deepen the community module without semantic change

Actions:

1. Move compatibility topology and Louvain behind the new facade.
2. Move incremental clustering, remapping, signatures, and base labels.
3. Route `pipeline`, `cluster_existing`, history, viewer model, and
   architecture projection through the facade or shared quality evaluator.
4. Remove duplicated orchestration and cohesion implementations.
5. Add typed errors and explicit work accounting.

Done when:

- compatibility results remain byte-equivalent;
- callers no longer sequence detector internals;
- the complete result is the test surface; and
- graph/output ownership remains unchanged.

Suggested commits:

1. `refactor(graph): introduce community result facade`
2. `refactor(graph): move compatibility topology and detector`
3. `refactor(core): consume complete community results`
4. `refactor(output): reuse community quality evidence`

### Phase 3: add typed topology as a candidate

Actions:

1. Add the typed Base Graph adapter.
2. Add the exhaustive relationship strength table.
3. Add confidence and bounded multiplicity contributions.
4. Add reciprocal-direction aggregation and evidence summaries.
5. Run a bounded policy matrix against Phase 1 qualification.
6. Freeze the best justified weights as `typed-evidence-undirected/v1`.

Done when:

- typed relationships are never silently defaulted;
- every topology edge is explainable by bounded evidence counts;
- Base Graph artifacts are byte-identical; and
- the candidate meets planted, real-repository, time, and memory gates.

Suggested commits:

1. `feat(graph): project typed community topology`
2. `feat(graph): weight relationship and confidence evidence`
3. `test(graph): qualify bounded topology policies`
4. `docs(graph): freeze typed topology v1`

### Phase 4: implement and qualify native Leiden

Actions:

1. Extract the common deterministic local-moving kernel.
2. Implement connected refinement and aggregate propagation.
3. Add independent connectedness validation.
4. Add oracle, articulation, resolution, tie, and limit tests.
5. Compare Leiden and compatibility Louvain on identical topologies.

Done when:

- every selected Leiden community is connected;
- deterministic and limit gates pass;
- quality improves on the targeted fixtures; and
- performance stays within the acceptance envelope.

Suggested commits:

1. `refactor(graph): isolate deterministic local moving`
2. `feat(graph): add native Leiden refinement`
3. `test(graph): verify Leiden connectivity and determinism`
4. `docs(graph): record Leiden qualification`

### Phase 5: add bounded candidate selection

Actions:

1. Distinguish omitted resolution from explicit `--resolution` in CLI parsing.
2. Generate the three rational auto candidates.
3. Implement common-resolution evaluation and lexicographic selection.
4. Record rejection and tie-break evidence.
5. Remove new-profile dependence on the legacy split passes.
6. Run full quality and performance qualification.

Done when:

- explicit resolution produces exactly one candidate;
- automatic work never exceeds three candidates;
- selector output is deterministic and inspectable; and
- the performance gates decide whether auto selection may become default.

Suggested commits:

1. `feat(graph): evaluate bounded resolution candidates`
2. `feat(graph): select partitions from quality evidence`
3. `feat(cli): preserve explicit fixed resolution intent`
4. `test(graph): qualify automatic partition selection`

### Phase 6: preserve incremental quality

Actions:

1. Replace the induced-only local topology with locked frozen-community anchors.
2. Add full-topology merged-partition evaluation.
3. Add explicit quality fallback reasons.
4. Add repeated edit/revert/rename/delete sequences.
5. Measure one-file update time and RSS on pinned corpora.

Done when:

- frozen external influence participates in local selection;
- a degraded local result always falls back or fails explicitly;
- unaffected assignments remain frozen when the local result qualifies; and
- incremental performance retains its documented advantage.

Suggested commits:

1. `feat(graph): retain frozen influence in local clustering`
2. `feat(graph): guard incremental partition quality`
3. `test(graph): cover incremental drift and reversibility`
4. `docs(graph): publish incremental quality measurements`

### Phase 7: coordinated default cutover

Actions:

1. Freeze algorithm, topology, quality, selector, and limit identities.
2. Thread those identities through build state, configuration digests, and
   immutable history profiles.
3. Switch normal, cluster-only, viewer, and historical materialization callers
   together.
4. Publish `compass.community-quality/1` if its contract has been accepted.
5. Update command, output, concept, compatibility, migration, changelog, and
   performance documentation.
6. Run the complete repository baseline and matching product gates.

Done when:

- no caller silently uses default Louvain;
- old output is coherently invalidated and rebuilt;
- historical realizations remain immutable;
- every public consumer validates the same profile identity; and
- the final qualification report satisfies all acceptance gates.

Suggested commits:

1. `feat(history): version complete community profiles`
2. `feat(graph): adopt qualified community profile`
3. `feat(output): publish community quality evidence`
4. `docs: document community detection cutover`

### Phase 8: cleanup after one release line

Actions:

1. Remove obsolete orchestration helpers and duplicated density code.
2. Make compatibility facades crate-private unless a supported library caller
   still requires them.
3. Retain only code required to read immutable prior artifacts; do not emulate
   old builds implicitly.
4. Re-run size, time, RSS, and determinism qualification.

Done when:

- the community module has one production interface;
- algorithm policy is not duplicated in CLI, core, output, or viewer code; and
- compatibility documentation matches retained behavior.

## Verification matrix

Every Cargo command must use this worktree's dedicated target directory under
`/Volumes/Workspace/crabbuild-target`.

### Narrow checks

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-c112 \
  cargo test -p compass-graph --locked

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-c112 \
  cargo test -p compass-core --test code_graph_v1_determinism --locked

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-c112 \
  cargo test -p compass-cli --test history_cli --locked

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-c112 \
  cargo test -p compass-cli --test viewer_export_cli --locked
```

### Product and graph gates

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-c112 \
  cargo test -p compass-cli --test compass_product --locked

sh scripts/check_product_boundary.sh
./scripts/qualify_code_graph_v1.sh --fixtures-only
```

The code-graph qualification runner must gain a clustered community-quality
mode; its existing extraction-focused no-cluster mode remains unchanged.

### Baseline before rollout

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-c112 \
  cargo fmt --all -- --check

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-c112 \
  cargo clippy --workspace --lib --bins --locked -- -D warnings

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-c112 \
  cargo test --workspace --lib --bins --locked
```

Viewer tests are required when a new quality artifact or visualization is
published:

```bash
npm ci
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
```

## Rollback

Phases 0 through 6 are additive or internal and can be reverted independently
while the compatibility profile remains the default.

After the Phase 7 cutover:

- do not rewrite published current or historical artifacts;
- restore a previous default only with a new engine/profile identity;
- invalidate current output through the normal configuration digest;
- document the membership change; and
- publish a new qualification report.

Rollback never means interpreting one profile's memberships as another
profile's result.

## Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Typed weights encode arbitrary preferences | Freeze only after planted and real-corpus policy qualification; publish the table and evidence mix |
| Multi-resolution multiplies cost | Cap at three candidates; explicit resolution uses one; withhold auto default if performance fails |
| Leiden implementation is complex | Keep modularity unchanged, add oracle fixtures, validate connectedness independently |
| Conductance favors trivial partitions | Use it only after integrity and modularity plateau gates, never as a sole objective |
| Package agreement becomes circular truth | Treat it as supporting review evidence, not the detector's input or only oracle |
| Incremental state affects history | Exclude prior assignments from historical selection and content identity |
| New profile causes community-ID churn | Version the complete profile, rebuild coherently, document migration, preserve immutable history |
| Quality artifact is mistaken for truth | Publish a metric vector, witnesses, limits, and explicit hypothesis language |
| Structural refactor hides semantic changes | Require compatibility byte-equivalence before typed topology or Leiden commits |

## Research basis

- Traag, Waltman, and van Eck,
  [From Louvain to Leiden: guaranteeing well-connected communities](https://www.nature.com/articles/s41598-019-41695-z)
  motivates the connected refinement phase and independent connectivity gate.
- Fortunato and Barthélemy,
  [Resolution limit in community detection](https://pmc.ncbi.nlm.nih.gov/articles/PMC1765466/)
  motivates bounded multi-resolution qualification instead of treating one
  modularity optimum as universal.
- Lancichinetti, Fortunato, and Radicchi,
  [Benchmark graphs for testing community detection algorithms](https://arxiv.org/abs/0805.4770)
  motivates heterogeneous planted-partition fixtures.
- Leicht and Newman,
  [Community structure in directed networks](https://arxiv.org/abs/0709.4500)
  motivates making direction projection explicit rather than silently
  discarding it.

These papers inform the design. Compass's contract remains defined by native
implementation, fixtures, pinned qualification, and published versioned
profiles.

## Related pages

- [Architecture graph hardening technical design](architecture-graph-hardening-phased-technical-design.md)
- [Architecture graph hardening qualification](architecture-graph-hardening-qualification.md)
- [Extraction pipeline](extraction-pipeline.md)
- [Workspace tour](workspace-tour.md)
- [Graph model](../concepts/graph-model.md)
- [How Compass works](../concepts/how-it-works.md)
- [Design principles](../design/principles.md)
- [Compatibility ledger](../../COMPATIBILITY.md)
- [Performance qualification](../../PERFORMANCE.md)
- [Community detection qualification](community-detection-quality-qualification.md)

**Next step:** retain fixed-resolution Leiden in production and complete the
pinned real-repository performance matrix before proposing automatic selection.
