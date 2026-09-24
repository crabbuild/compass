# Output reference

Compass outputs range from the current `compass-out/` directory to versioned
CompassQL results and immutable history exports. This reference describes
consumer responsibilities and authority.

## Current output directory

Default:

```text
compass-out/
├── graph.json
├── graph.html                   # unless omitted by size or --no-viz
├── GRAPH_REPORT.md
├── orientation.json              # clustered Agent Orientation
├── manifest.json
├── program.json                 # only with --program or --program-artifact
├── graph-overview.json          # clustered builds
├── community-quality.json       # clustered typed builds
├── cache/                       # Compass-owned disposable cache layout
├── current-snapshot
├── snapshots/<current>/
│   ├── graph.json
│   ├── graph.html, report, manifest, and optional public artifacts
│   ├── store.ref                # with the default SQLite query index
│   ├── build-state.json
│   ├── output-stats.json
│   ├── ast-fact-digests.json
│   ├── analysis.json and labels.json  # clustered builds
│   ├── labels.json.sig     # when label signatures are available
│   ├── semantic-marker.json # semantic builds and history exports
│   ├── learning.json       # learned reflection overlay, when present
│   ├── cache/              # operation-specific disposable graph caches
│   │   ├── graph.json.query-v1.cache
│   │   ├── graph.json.affected-v1.cache
│   │   ├── graph.json.traversal-v1.cache
│   │   └── graph.json.<digest>.content-v1.cache
│   └── source-root.txt
├── store/
│   └── store.sqlite3   # with the default SQLite query index
├── root-artifacts-complete
├── cached.json             # cache-check hits, when any
├── uncached.txt            # cache-check misses
├── obsidian/sync-manifest.json # when exporting an Obsidian vault
└── source-inventory.json   # versioned-history export, when requested
```

`--out DIR` or compatible `COMPASS_OUT` use can select another root.

The ordinary files at the root are a stable, flat consumer façade. Compass
publishes its immutable snapshot first, then materializes each root file by
atomic replacement; a completion marker makes an interrupted façade update
self-repair on the next build. Compass-aware readers continue to resolve the
current snapshot, while browsers, scripts, archive tools, and integrations
can use literal paths such as `compass-out/graph.json` and
`compass-out/graph.html`. Consumers that need a related set should read it only
after the producing Compass command returns successfully.

The output root already establishes Compass ownership, so entries beneath it
use concise purpose-based names without repeating a `compass-` prefix. The
`cache/` directory is rooted beside these files for familiar incremental-build
ergonomics, but its contents and encoding are private to Compass. Do not copy
another product's cache or manifest into it. Future storage and cache
revisions can evolve independently without changing the flat public artifact
paths.

## Authority table

| Artifact | Authority | Consumer use |
| --- | --- | --- |
| `graph.json` | machine-readable graph snapshot | queries, integrations, export |
| `store/store.sqlite3` | bounded shared namespace/partition/key query index | default large-graph queries and explicit store-engine queries |
| current snapshot `store.ref` | typed selector for the co-published store identity and snapshot | store-engine validation before query execution |
| `program.json` (optional) | provenance-aware Program IR | program inspection, semantic analysis |
| `GRAPH_REPORT.md` | derived human orientation | architecture survey |
| `orientation.json` | versioned Agent Orientation bound to the same graph generation | coding assistants and MCP |
| `community-quality.json` | strict graph-bound community evidence | detector inspection, qualification, immutable history |
| `community-hierarchy.json` | strict graph-bound community hierarchy with a level budget | bounded community overview, level navigation |
| `graph.html` | derived optional visualization | interactive exploration |
| `manifest.json` | incremental build state | next compatible update |
| binary query caches | disposable acceleration | internal query loading |
| semantic sidecars | depends on artifact class | completeness/evidence/export |

Do not reconstruct graph truth from HTML when JSON is available.

`store/store.sqlite3` is the default local SQLite realization
shared by retained graph snapshots. It is addressed through the `compass-store`
namespace/partition/key contract and is not a public SQL schema. The file is
not copied into a published snapshot; a new build writes immutable content,
checkpoints it, and publishes a digest-bound snapshot reference. `graph.json`
remains the complete portable graph engine. Pass `--store json` during a build
to omit the sidecar; `--engine json` forces the portable reader, while the
default query engine uses the sidecar when it is present and fails closed if
its reference is corrupt.

The store snapshot accepts canonical graphs up to 2 GiB. This larger, still
finite bound applies only to the indexed store path; in-memory JSON readers
retain their independent 1 GiB cap and should be used only for bounded
investigations or smaller outputs.

## `graph.json`

Top-level node-link shape:

```json
{
  "directed": true,
  "multigraph": true,
  "graph": {},
  "nodes": [],
  "links": []
}
```

### Node

```json
{
  "id": "opaque-stable-string",
  "label": "authorize_payment()",
  "file_type": "Function",
  "source_file": "src/payments.py",
  "source_location": "L12",
  "community": 4
}
```

Only `id` is structurally required by the typed node record. Attributes are
extensible.

For a source-backed declaration with one containing owning scope, the node's
`source` anchor spans the complete definition and is the authoritative range
for editor navigation. Its AST provenance keeps the narrower exact declaration
anchor, such as the identifier token. When a containing definition extent is
missing or ambiguous, Compass publishes the exact declaration anchor instead
of selecting an arbitrary scope.

### Edge

```json
{
  "source": "caller-id",
  "target": "callee-id",
  "relation": "calls",
  "confidence": "INFERRED",
  "context": "call"
}
```

Source/target IDs must be indexable. Attributes are extensible.

Compass sets `multigraph` from the emitted links. It is `true` when two links
share an endpoint pair (ordered for directed graphs, unordered for undirected
graphs), including repeated self-loops. Consumers do not need to request this
promotion.

## `community-quality.json`

Clustered typed builds publish schema `compass.community-quality/1`. The
artifact records the exact `graphGeneration` and SHA-256 `graphDigest`, the
algorithm/topology/quality/selector/seed/limits identity, the numeric limits,
partition metrics, per-community evidence, candidate summaries, bounded
witnesses, exact omissions, and `resultDigest`.

Consumers must reject unknown schemas or fields and call the equivalent of
`validate_for_graph` against the selected canonical `graph.json`. A digest,
generation, or profile mismatch means the files are not one coherent artifact
set. `resultDigest` detects mutation of the quality payload itself. A missing
artifact is valid for an older graph, a schema-less legacy recluster, or a
`--no-cluster` build and means quality evidence is unavailable.

Metrics form a vector rather than a pass/fail truth label. Modularity is
reported at the named evaluation resolution; conductance, connectedness,
largest-community fraction, singleton count, topology evidence mixes, and
witness omissions must be interpreted alongside it. Numeric community IDs are
local to this graph realization.

## `community-hierarchy.json`

Clustered typed builds publish schema `compass.community-hierarchy/1` beside
the quality artifact, bound to the same `graphGeneration` and SHA-256
`graphDigest`. A flat partition scales with the repository, so an overview
needs a bounded number of named, nested units: measured examples are 112
communities for `pallets/flask` and 2,781 for `colinhacks/zod`.

The artifact is a list of levels, coarsest first. Level 0 is the root a reader
opens; the last level is the published partition itself, where group `i` pairs
with the community id recorded in `groups[i].community`. Community ids are not
always dense — the incremental path remaps surviving communities — so the
pairing is explicit rather than positional. Levels never repeat node ids:
`groups[*].childIndices` index the next finer level, `memberCount` sums the
members below, and `finestSignature` digests the published partition's member
signatures, so a reader can prove the hierarchy describes the partition it was
built from without storing per-node membership.

The budget tuple is `rootTarget` (24), `levelTarget` (300), `maxLevels` (4), and
`minLevelResolution` (0.05), published with the identity
`community-hierarchy-budget/v1`. `budgetSatisfied` is a recorded fact, not a
promise: a repository can publish communities that share no evidence at all,
and the artifact keeps the achieved count instead of merging them.

Each level records the rule that produced it. A `relationship` level comes from
the same seeded Leiden local moving clustering uses, at the resolution in
`resolution`, and must remove at least a tenth of the level below it to be
published. A `locationAffinity` level is cut out of the directory tree its
groups already cite: the cut starts at the repository root and repeatedly
expands the largest directory whose children still fit the budget, so every
merged group is a real directory its members share, and a group that cites no
dominant directory stays a group of its own. `mergeEvidence` records the
counts, including `unkeyedGroups`. The policy identity is published as
`mergePolicy`.

Group labels carry their provenance in a fixed order: `dominantDirectory`
(longest common directory prefix covering at least 60% of members),
`modulePrefix`, `hubMember`, then a generic `communityId`. Consumers must read
`label.rule` and `label.generic` rather than parsing label text. Group
`quality` reports cohesion and conductance over the graph that level
partitions, plus `boundaryKinds` counted from the exact kind set recorded in
`boundaryKinds`.

Level membership is a navigation aid derived from the same evidence as the
communities themselves. It never changes nodes, edges, or query results, and
absence of the artifact means unavailable navigation, not an empty hierarchy.
`compass export hierarchy-json` reproduces the published artifact unchanged and
refuses an unknown schema major or a mismatched graph.

The standalone page embeds the same levels as
`compass.viewer.hierarchy/1`, so an export with a published hierarchy opens on
level 0 instead of a derived overview. The graph toolbar's scope reads
`Level 0 | … | Symbols`: switching a level redraws the canvas, the coupling
matrix, the area map, or the tiers from that level's projection, and the
`Symbols` scope still shows the underlying node set. Double-clicking a group
descends one level, narrowed to that group's children; the breadcrumb and the
`Overview` control walk back up one group or to the repository. A level the
export could not draw inside its node budget renders as the overview the export
already had. Exports without the artifact keep the previous behaviour exactly.

### Group identity and the identity ledger

`groups[*].id` is evidence-derived: `h<level>-<signature16>`, digesting the
sorted member-community signatures of the group's members, with the algorithm
recorded as `signatureAlgorithm` (`hierarchy-signature/v1`). `signature` is the
group's digest in this build; `id` is the durable name, which reconciliation
rewrites to the previous build's id when the same group survives. `index` stays
presentation order, so a reader must key on `id`.

Every rebuild publishes `community-hierarchy.json.sig` beside the artifact: a
ledger of flattened group ordinal to `"<id> <signature>"`. It is written with
the other required artifacts, removed with `--no-cluster`, and lets a later
build restore identity when only the ledger survives. Reconciliation events are
bounded and are not embedded in the artifact; consumers read them from the
reconciliation report or the history comparison.

### `compass.community-hierarchy-diff/1`

Comparing two history realizations publishes a bounded diff of their community
hierarchies on the history workbench view (`compass.viewer.workbench/1`,
`kind: "history"`, `hierarchyDiff`). It names both sides (generation, graph
digest, hierarchy digest, level and group counts) and the policy that produced
it, then counts `stable`, `split`, `merged`, `appeared`, `disappeared`, and
`ambiguous` entries with a bounded `events` list.

An event carries the base and target group ids it involves, the member overlap
that justifies it, and the members at stake. A group whose id survives is
stable and is only listed when its membership moved underneath the same id;
where an id does not survive, the members decide — one base group reappearing
across several target groups is a split, several folding into one is a merge,
and a base group with two candidates inside the ambiguity margin is reported as
ambiguous with both named. `omittedEvents` counts the entries the bound
withheld, and `resultDigest` covers the payload. Consumers must reject an
unknown schema major, and absence means the comparison is unavailable because a
side published no hierarchy — never "nothing changed".

### Inference levels

Graph-building commands accept `--inference-level low|medium|high|max`. The
levels are deterministic and nested:

| Level | Published relationship evidence |
| --- | --- |
| `low` | exact relationships only |
| `medium` | `low`, plus inferred relationships whose endpoints are both source-backed |
| `high` | `medium`, plus explicitly qualified external relationships anchored in source syntax |
| `max` | all retained inference, including deferred-receiver relationships |

`low` is the default. `max` remains an explicit opt-in that preserves the
former complete graph behavior. Lower levels filter after evidence
normalization, prune unreferenced inferred placeholder nodes, and keep every
retained edge endpoint valid. The selected level is part of the build profile
and configuration digest, so an output built at one level is not reused as
though it represented another. This policy is intentional selection, not a
publication omission.

Inference controls graph breadth, not source anchoring. An inferred edge may
still have an exact relationship site while its target identity remains
unproven. Use `compass diagnose quality --json` to inspect exact/inferred ratios
for the selected output.

### Consumer requirements

- preserve unknown attributes;
- treat IDs as opaque strings;
- preserve direction;
- preserve parallel edges when multigraph is true;
- do not make JSON member order meaningful;
- use canonical/semantic equivalence for graph comparisons;
- validate file size and JSON at your trust boundary.

Compass readers use a bounded 1 GiB default graph-size cap. This accommodates
qualified enterprise artifacts while preventing unbounded input reads.
Operators can set `COMPASS_MAX_GRAPH_BYTES` to an explicit byte count or
`<N>MB`/`<N>GB`; raising it also raises the memory exposure of JSON decoding
and indexing.

That whole-JSON reader cap is separate from the current SQLite graph-index
snapshot used by `--store sqlite`. The graph-index has no aggregate canonical
payload or record-count limit: it publishes content-addressed tree objects of
at most 256 KiB through write batches of at most 16 MiB, and bounds each point
or range query independently. Consumers that request a whole-graph export can
still encounter the materialized-read record budget and should use indexed
queries for substantially larger repositories.

`compass store status`, `validate`, `backup`, and `restore` also remain on the
large-graph path. They stream file digests through fixed-size buffers and
validate the selected manifest plus every reachable immutable tree object;
they do not require a `COMPASS_MAX_GRAPH_BYTES` override.

### Partial publication diagnostics

A successful build can publish a strictly valid partial graph after
quarantining invalid individual records. The durable warning codes are:

- `publication_omitted_node`
- `publication_omitted_edge`
- `publication_identity_collision`
- `publication_omission_summary`

The first three provide bounded examples. The summary contains exact omitted
node, omitted edge, identity-collision, and capped-example counts. At most 100
examples of each record category are stored.

The Compass-owned `output-stats.json` and sealed build state retain the same
counts so no-op and watch results preserve the partial status. They are
operational state, not an alternative graph schema. Consumers should use graph
diagnostics or typed query `incomplete_coverage` diagnostics.

## `GRAPH_REPORT.md`

The report can include:

- corpus and graph summary;
- freshness/build metadata;
- god nodes;
- communities;
- surprising connections;
- cycles/diagnostics;
- suggested questions, including bounded structural-gap questions when two
  well-formed communities share topical two-hop evidence but lack a direct
  topical relationship, and disconnected-component questions for multiple
  source-backed graph islands.

It is intended for people and can evolve in prose/format. Do not parse it when
structured data or command JSON exists.

Structural-gap questions are investigative evidence, not newly inferred graph
edges. Compass dampens shared intermediaries by their degree, excludes
containment/import/wiring relations from topical linkage, and ignores
file/concept/JSON-key-only noise. The report may therefore ask what would
connect two communities without asserting that a connection exists.

## Typed graph-insights projection

Clustered `analysis.json` includes a bounded `blindSpots` value with schema
`compass.graph-insights/1`. It contains ranked `communityGaps` and, when more
than one source-backed component exists, `disconnectedComponents`. Each gap
retains stable anchors, shared-intermediary witnesses, direct topical-edge
witnesses, and exact counts; each component retains bounded member witnesses.
`omissions` and `limits` are part of the contract, so a missing item is never
silently interpreted as evidence that no item existed.

The same projection is included as optional `blindSpots` in
`compass.orientation/2`, rendered in `GRAPH_REPORT.md`, and exposed through
the read-only MCP resource `compass://graph-insights`. The projection does not
add, remove, or rewrite graph edges. `compass history blind-spots --format
json` compares these exact IDs across immutable realizations; realizations
without the sidecar are counted as observations without graph insights.

Community evidence labels prefer a meaningful symbol or document heading over
Markdown semantic table navigation records, even when a table container has more
structural edges. A community containing only table navigation records receives a
source-anchored `Table (path:line)` label. When other communities share a hub
name, Compass adds a compact source or wiring-site anchor and, only if needed,
the graph-local community ID. These labels are deterministic navigation aids,
not community identity; consumers that need identity should use the community
ID and member set instead.

The architecture view is a separate, versioned projection rather than a
presentation alias for raw communities. It classifies Production and All-code
source scopes before grouping, derives project-specific owner and subsystem
names from source paths, declarations, and optional overlays, and keeps names
separate from stable membership-derived IDs. Relation classes and lenses are
explicit in the model. Large graphs remain bounded by an overview whose exact
omission counts link to the searchable group directory; omitted groups are
never merged into a synthetic `Other` subsystem or connected to invented
routes.

Production excludes Test, Generated, Vendor, Documentation, and Unknown
sources before grouping. All-code retains them with exact counts. Memberships
use validated `nodeIndex` and `groupIndex` references into deterministic arrays
to keep large payloads bounded without discarding drill-down data. Extraction
completeness, overview omissions, and architecture quality are separate
signals.

The Architecture Map and Community Directory omit communities made entirely
of Markdown table navigation records. Those communities count toward the
report's omitted-community coverage and their source-backed nodes remain in
the graph; the report does not present parser partitions as architectural
subsystems.

The report begins with a bounded Agent Orientation for first-session or broad
repository context. `orientation.json` is the versioned machine form of that
same fitted model. Compass publishes both from one coherent build input and
validates the graph generation and exact streamed `graph.json` digest before
`compass export orientation-json` or
`compass://orientation` returns it. `compass://report` renders the human report
from that validated model; it never trusts an adjacent Markdown file by name.

The Architecture Map highlights up to twelve leading communities. The later
Community Directory is the broader navigation index: it ranks every non-empty
community, including thin communities, by member count and connectivity, shows
as many as fit within the 256,000-character report bound (up to 4,096), and
splits presentation into two tiers. The top 32 ranked communities receive full
detail with up to twelve high-connectivity entry points and four strongest
links per direction. Every remaining retained community receives a compact
one-line index entry containing its rank, evidence label, exact query scope,
member count, cohesion, connectivity, and best source-anchored entry point.

The compact orientation remains bounded to 16,000 characters, while its JSON
and the MCP resource transport share a 4 MiB envelope. Every bounded list
reports exact shown and omitted counts. Headings and boundary links prefer
evidence labels; graph-local numeric IDs remain visible as `community:<id>`
query scopes where an agent needs an exact follow-up command.

Human-facing Markdown prefers `label — source:range` references. When duplicate
labels have distinct source anchors, Compass omits redundant node IDs. Opaque
IDs longer than 48 characters are shown only as a bounded prefix/suffix plus a
deterministic fingerprint when an ID is still needed for disambiguation. Exact
IDs and exact query argv remain unchanged in `orientation.json`; query argv
longer than 240 rendered characters are referenced there instead of being
duplicated into Markdown.

## `graph.html`

Optional interactive visualization. It may be absent when:

- `--no-viz` was used;
- graph size exceeds a rendering limit;
- a specific build/export omitted it.

It is not required for query commands.

The document is self-contained and uses the same versioned graph workbench as
the VS Code extension. Loading and exploring it performs no runtime network requests, follows the
operating system's light or dark color scheme, and retains keyboard, reduced
motion, narrow-screen, and high-contrast behavior from the shared viewer.

Double-clicking a source-backed node, edge, or inspector source card opens the
file and highlights its recorded lines in the VS Code extension. A standalone
HTML export instead opens an immutable forge permalink when all required
evidence is available: the graph records a full source commit, the graph is
inside a Git worktree with a recognized `origin`, and that origin is GitHub,
GitLab, or Bitbucket. Compass uses the recorded commit when it is reachable
from a local `origin` remote-tracking ref. For a local-only commit, Compass may
instead use an immutable published common ancestor, but only after a bounded
Git comparison proves every source path represented by the graph has identical
content at both commits. This preserves exact files and line anchors while
avoiding dead forge URLs for local metadata-only commits. Historical
comparisons use the commit for the selected side. If any evidence is absent,
unsafe, or source content differs, the viewer does not invent a link and
explains that the local source can be opened from the VS Code extension.
No repository URL is added to `compass.viewer.workbench/1` or
`workbench-json`; standalone HTML carries the optional presentation metadata
separately.

The embedded `compass.viewer.workbench/1` model contains an ordered list of
independently bounded views with explicit `complete`, `summary`, or `partial`
coverage. One HTML file can contain code, call, impact, affected, architecture,
history-comparison, and artifact-specific lenses. Its navigation rail keeps
the current graph identity and exposes hash links such as `#view=impact-run`.
Both the navigation rail and graph inspector can collapse independently, and
the repository title appears once in the navigation header so inspector space
starts with search and node details.
Graph lenses share relationship, evidence, node-kind, and language filters;
call, impact, and affected views start in a deterministic depth-layer layout.
Architecture views use subsystem routes, while history views overlay added,
removed, and changed graph evidence.

Node-link graph views provide bounded 1–4-hop selection isolation with exact
incoming, outgoing, or bidirectional traversal, adjustable layout spacing, and
a navigable minimap based on the rendered graph coordinates. Workbench graph
filters live in the top graph-control rail, which shares the view header row
instead of floating over the canvas, and open as a compact panel. In a narrow
header the rail wraps rather than scrolling its trailing controls out of reach.
Filters and their result count follow the graph currently on screen when moving
between an overview and community detail, and the community list stands down to
its summary line while one community is open so the inspector keeps the room
its node detail needs; it returns with the overview, and the reader can open it
by hand meanwhile.
Neighborhood depth and direction can be prepared before selecting a node;
isolation becomes available after selection and fits the resulting
neighborhood. The graph-settings panel documents keyboard controls; press `?`
while focus is outside a text or selection control to open it. Pause and resume
are explicit labeled actions; resuming a settled layout reheats it enough to
remain visible at a fitted overview scale.

The machine form is available from `compass export workbench-json`, or from
`compass export json` when at least one view is requested. Unknown major
workbench schemas must be rejected. Plain `compass export json` remains
`compass.viewer.graph/1` for existing consumers.

When the node limit selects a community overview, the standalone document
embeds a deterministic bounded detail for every community it can size: the
export spends at most 5,000 detail nodes and 40,000 internal detail edges on
one shared window, so small communities are embedded whole and large ones open
on their most connected symbols instead of being dropped. Members are ordered
by connectivity first, and a bounded window names itself in the viewer — it
states how many of the community's symbols it holds and points at the VS Code
graph or `compass export json --community ID` for the complete community. Only
an export whose budget cannot host a single member leaves communities marked as
unavailable for standalone drilldown. Details are validated only when opened.
Double-click a community node (or use **Open community** in the inspector) to
enter its member graph; use **Overview** to return. On a page that also opens
on a published hierarchy, the finest level's groups open the same details, so
descending the levels and entering a community stay one path. Embedded details
preserve internal edges, source anchors, and hyperedges, while cross-community
edges remain represented only in the overview.

Large community overviews use a deterministic hub-centered layout. Physics is
paused, labels remain bounded, and at most 4,000 aggregate edges are rendered
as straight hairlines. The visible edges form a deterministic strongest-edge
backbone; the inspector continues to report the complete relationship count
and discloses the rendered count. This keeps repositories with thousands of
communities from producing an expensive rectangular edge curtain without
changing `graph.json` or the complete overview model.

When an exported graph is large but not itself aggregated — for example a
repository under the 5,000-node export limit — the viewer derives the same
community overview from the embedded model instead of painting thousands of
unlabeled symbols on one screen. The overview packs one labelled bubble per
community, sized by exact member count, with cross-community relationships
weighted by the number of relationships they summarize and drawn in the colour
of their dominant relationship category. Hovering an aggregated relationship
states the exact mix (for example `12 calls · 4 imports`), so the overview
answers what binds two subsystems, not only that they are bound. The first
screen then reads as a map of the repository rather than a hairball.

Community overviews keep their labels readable in a fitted view: the packing
reserves label room for the communities that matter most — ranked by member
count, cross-community coupling, and boundary content such as routes and
database objects — the rest stay available on hover and in the inspector, and
**Show labels** reveals every bubble label. A `Repository` path above the canvas
shows where the reader is and returns to the overview from a community detail,
and every community row in the inspector can open its group directly.

The same communities can be read through four designs, switchable from the
**Overview design** control in the graph toolbar:

- **Bubbles** — the packed canvas map above; position and labels carry
  importance, edge colour carries the dominant relationship kind.
- **Matrix** — one row and column per community (bounded to the most important
  ones, with the omitted count stated), cell saturation is the exact
  relationship count and cell colour the dominant relationship kind; selecting a
  cell opens that community.
- **Area** — a strip treemap where every tile's area is exactly proportional to
  its symbol count, including a single disclosed tail tile when the render bound
  applies. This is the design that stays readable for repositories with
  thousands of communities.
- **Tiers** — importance tiers of proportional bars with coupling ribbons
  between them, which shows how the important layer couples into the rest.

Every design reads the same validated model and the same overview projection, so
switching designs never changes a query result, an artifact, or a community
identity. When an export records only relationship counts — the aggregated
fallback above the node limit — the matrix and tiers say so instead of implying
that a neutral cell colour means something.

Community colours come from one shared presentation palette
(`crates/compass-output/src/palette.rs`) used by the viewer model, the HTML and
SVG exports, and the Obsidian export, so the same graph looks the same wherever
it is opened. The twelve hues sit in a narrow lightness band with moderate
chroma: no community shouts, labels stay legible in ink or on white, and the
index alternates hue families so neighbouring communities rarely share a hue.
Colour is presentation only — it carries no meaning that a query depends on, and
changes to it never invalidate a graph, a community identity, or a cached
artifact.

The graph canvas is flat schematic paper: one surface colour plus a hairline
grid, with the community shapes, relationships, and labels carrying the
information. Light and dark operating-system themes, VS Code themes, and
high-contrast themes all drive the same tokens, so a standalone export and the
editor extension stay visually identical.

**Automatic** layout arranges itself when a view opens: the canvas starts from
its deterministic seeded map, runs the force simulation until it settles, and
stops by itself — symbol canvases, community overviews, and community
drill-downs all benefit, and the arranging screen offers "Show graph now" if a
graph takes longer than expected. Once settled, a deterministic separation pass
removes any bubble and label collisions the simulation left behind, so the
arrangement follows the couplings while labels stay readable. Graphs past the
interactive budget (1,000 nodes or 4,000 relationships) keep their deterministic
seeded map and say `press Layout to arrange` instead of blocking the first
frame. Choosing Circle, Concentric, Spiral, or Square grid places the seeded
layout immediately and never starts physics; **Layout** and **Stop** remain
explicit actions on the toolbar, and `F`, `+`, `−`, `0`, `I`, `[`, `]`, `D`,
and `M` keep working as documented in the graph settings panel.

Layout is centre-weighted. The community overview and the flat community map
both place the most important community in the middle and settle every next one
outward within its own radius, so large communities hold the centre while small
communities and single symbols scatter around the outside. Automatic layout adds
a second step after the force simulation: each settled node keeps the direction
its couplings gave it and moves toward the radius its importance rank earns
before collisions are separated, which is what keeps a 174-community map centred
and comfortable instead of drifting to one side.

Community overviews spend hue on signal rather than on everything. The
communities the importance budget labels keep their palette colour and a soft
halo in the same hue; the long tail renders as neutral context, and pointing at
or selecting a context bubble reveals its community colour on the spot. That is
what stops a 174-community map from reading as confetti while still letting a
reader find any community's colour, which the inspector's community list keeps
as the full key.

Standalone documents carry a **Colour theme** control — `Auto`, `Light`, or
`Dark` — in the graph toolbar. `Auto` follows the operating system, and pinning
a theme keeps the export's own surfaces stable for a screenshot or a shared
file regardless of the viewer's system. The control never appears in an editor
or IDE host, where the editor's own theme tokens take precedence over every
Compass token.

The control rail keeps the frequent actions and gives the rest a home. Scope,
design variant, layout, run layout, zoom, fit, graph settings, and any host
control (such as the workbench **Filters**) stay on the rail; **node labels**,
**relationship labels**, **fit selection**, and **reset view** live in the graph
settings panel, and `L` / `⇧ L` toggle labels from the keyboard. The design
switch carries one icon per design — scattered map, coupling grid, area map,
tier rows — and names itself on hover and to assistive technology rather than
spending rail width on a label. When the graph
stage is narrower than 1,240 px — an editor rail and an inspector are often
enough — the rail wraps onto a second row, the scope and design switches drop to
icons, and the breadcrumb and legend move down with it, so no control scrolls
out of reach. Every control keeps an accessible name whether or not its visible
label fits.
**Communities** and **Symbols** in the graph toolbar switch between the derived
overview and the unmodified symbol canvas; the graph remains the same validated
model and no artifact is rewritten.

Opening a community from a derived overview arranges that community once, then
settles into the usual paused layout. A community larger than the viewer's
drill-down budget opens its most connected symbols first and says so in the
view; search still reaches every symbol and opens the community that holds it.
**Overview**, the toolbar back control, or `Escape` returns to the overview.

The HTML DOM and CSS classes are presentation details, not a compatibility
contract. Automations should consume `graph.json` or `compass export json`
instead of scraping the viewer.

## `manifest.json`

The manifest supports incremental detection and cache compatibility. It
represents the artifact set it was published with.

Do not:

- edit it manually;
- copy it between unrelated roots;
- pair it with another graph version;
- treat it as a durable historical graph.

A forced/cold build can regenerate current output.

## `program.json`

`program.json` is the optional canonical, language-neutral Program IR produced
by native `init`, `ensure`, `update`, `extract`, and `watch` builds when `--program` or
`--program-artifact` is selected. Its public schema identifier is:

```text
http://crab.build/compass/v1
```

The artifact records providers, evidence, modules, functions, operations,
resolved and unresolved calls, capability coverage, and derived summaries.
Coverage is explicitly `complete`, `partial`, `indeterminate`, or `failed`;
consumers must preserve non-complete reasons and must not interpret unresolved
calls as proof that no target exists.

Use `compass program` for read-only inspection and CompassQL projection.
Reject unknown schema identifiers rather than guessing compatibility.

A verified managed Python provider has an ID of the form
`scip-python:<profile-sha256>:<artifact-sha256>`. Its companion SCIP manifest
uses `compass.scip-manifest/1` and may carry the additive
`managed_analyzer` object with schema `compass.managed-analyzer-profile/1`.
The frozen environment is validated and hashed into provider/cache identity,
but environment paths and platform details are not copied into Program facts.
Only complete, offline profiles are accepted; unknown profile majors and
timeout, cancellation, permission, partial, failed, or stale states fail
explicitly.

## Query text

`query`, `path`, `explain`, `affected`, and some history commands emit
human-readable text. It is stable enough for people, not the preferred machine
contract.

Natural-language `query` output distinguishes declaration locations (`src` and
`loc`) from unresolved-symbol occurrence sites (`wiring`) and relationship
occurrences (`at`). `explain` similarly reports `Source` for declarations,
`Wiring` for source-less placeholders, and source sites on connections.

When exact automation is required, use:

- CompassQL JSON/JSONL;
- history JSON;
- diff JSON;
- direct graph JSON.

### Agent Query View

The focused query commands and MCP query tools also expose the strict,
bounded projection `compass.query.agent-view/1`. It is intended for coding
agents that need to decide whether a result is usable before reading all graph
evidence. The projection is derived from the authoritative raw response; it
does not run another resolver or change ranking, direction, provenance, or
limits.

```text
RESULT
ANSWER
CAVEATS
PRIMARY RESULTS
PATHS
RELATIONSHIPS
NEXT ACTIONS
DETAILS
```

The JSON form has `status.resultState` (`answered`, `candidates`,
`needs_resolution`, `no_match`, or `no_path`), separate match/evidence and
execution states, explicit caveats, full stable IDs, source locations, and
`identity.sourceResultDigest` plus `identity.viewDigest`. A no-match or
ambiguous response is never presented as a positive answer. `coverage` is
`incomplete` only when the raw query says so; otherwise it is `unknown`.

When a typed lookup cannot resolve one exact target, the projection retains the
exact-name candidates in `primaryResults` with their IDs, kinds, and source
anchors, reports `status.matchState = ambiguous`, and emits
`retry_with_exact_id` actions. Callers therefore disambiguate in one follow-up
instead of issuing a broad search. Primary results are deduplicated by node ID,
including when a real self-edge names the same node twice.

Typed text output is paged. Each page carries a
`Pagination: page=N range=A-B of T next=<CURSOR>` footer; `--cursor` continues
the same ledger at the same `--text-budget`. The cursor is a checksummed
base64url envelope with a compact wire form that binds the operation, graph
identity, page number, and a digest of the reviewed entry prefix at 64 bits
each; cursors from an earlier release are rejected with an explicit version
error. A cursor from another graph, another operation, or a changed result
fails closed rather than restarting the page. One page renders at most 12
primary results, 24 relationships, and 5 paths while reporting the ledger's
true total, so a page carries the strongest evidence and `next=` continues the
rest. Stable identifiers are printed only for a non-exact match, where the
printed name may not address the row; a resolved answer and an exact-name pick
list print the qualified name and source anchor instead, and the raw
`compass.query/1` response still carries every identifier. A `PATHS` row prints
its hop count and the labelled trail rather than the path identity, which is
built from every node identifier on the trail, and a relationship row keeps its
relation, endpoints and site on one line, spelling out the confidence and
resolution only when they are not the strongest (`exact`).

`--format agent-json --brief` emits `compass.query.agent-view.brief/1`: the same
status, headline, caveats, source-located entities, relationships, paths, and
next-action argv as `compass.query.agent-view/1`, without `identity`,
`omissions`, per-relationship IDs, per-entity roles, or per-edge evidence
layers. The brief projection is presentation-only; exact record identity and
digests remain in the raw `compass.query/1` response.

Agent View relationships are ordered by relation strength so a bounded answer
keeps the direct usage an agent asked for: calls, instantiations, routes,
handlers, and registrations first; then imports and exports; then references
and documents; then remaining relations, with the exact relationship ID as the
deterministic tie-break. Callers and callees primary results follow the same
order, and their headline reports the source response's edge count, so
`omissions.relationships` shows how many of them the bounded projection left
out.

`compass impact` output follows the same evidence rule in two places. The
reverse walk visits edges that name the expanded node before edges that only
reach its containing owner, then ranks by relation strength, because the
retained trail ledger is capped and a heavily referenced symbol would
otherwise spend it on owner-level trails. The text and agent views then order
the impacted nodes by trail length and the strength of the trail's last hop, so
the direct callers a change breaks are listed before the symbols that only
touch a containing owner.

The fixed presentation profile retains at most 12 primary results, 24
relationships, 5 paths, 16 caveats, and 5 next actions. Serialized JSON is
limited to 256 KiB and text to 64 KiB. `omissions` and
`projectionTruncated` make projection loss explicit; raw JSON remains the
complete audit result. Human text may evolve, so automation should consume
Agent View JSON or the raw versioned response rather than parse headings.

## CompassQL JSON

Schema:

```text
compass.cql.result/1
```

Contains:

- explicit version tag;
- columns;
- typed rows;
- optional plan;
- optional profile.

Reject an unknown major version.

## CompassQL JSONL

Schema:

```text
compass.cql.jsonl/1
```

Order:

```text
header
row object
row object
...
summary
```

Do not treat a truncated stream without a successful command/summary as a
complete result.

## Atomic query output

`--output PATH` writes a completed rendering atomically. On compile, graph-load,
execution, limit, cancellation, or output failure, no successful partial result
should appear at the final path.

Consumers should still check exit status before opening the file.

## History JSON

History commands that accept `--format json` expose stable structured status,
list, show, build, preference, or GC results. Exact fields are defined by the
current history schema and tests.

Record:

- commit;
- realization ID;
- fingerprint;
- preferred/validation state;
- schema/version.

## Diff JSON

```bash
compass diff OLD NEW --format json
```

Uses schema `compass.semantic_diff.report/1`. The report contains ranked
semantic findings, affected callers/modules, source and graph evidence,
verification state, completeness, and a collapsed-finding summary. Routine
symbol churn is collapsed unless `--all` is supplied. Default text output
shows 20 findings per section and reports every hidden count; `--limit N`
changes that budget, while JSON and `--all` are exhaustive. Normal diff
requires compatible build profiles.

`verification.state` is `covered`, `gap`, `partial`, or `unknown` for the
static MVP (runtime adapters may also report `stale`, `failing`, or `not_run`).
Compass reports a test gap only when the available evidence can establish one;
missing or incomplete evidence is not presented as proof of a gap.

## Diff HTML

```bash
compass diff OLD NEW --format html --output semantic-diff.html
```

Writes one self-contained HTML document with no runtime server or external
assets. It includes the complete `compass.semantic_diff.report/1` JSON payload,
actionable metrics, feature groups, finding search and filters, expandable
evidence, affected consumers, verification state, completeness, limitations,
and collapsed routine-change groups. The Code section uses the pinned
`@pierre/diffs` 1.2.12 renderer for line numbers, intraline emphasis, hunk
metadata, line wrapping, and unified/split layouts. Compass embeds the library
in the document, so the report has no CDN or runtime dependency, and retains
the exact Git patch as a fallback if script execution is unavailable. The
Graph section contains a compact changed-subgraph visualization plus
exhaustive added, removed, and changed node/edge lists. Non-semantic graph
metadata churn is summarized separately, including location/layout fields and
edge-identity shifts that preserve multigraph multiplicity. HTML output always
requires an explicit path; `compass export html` remains the full graph
renderer and does not accept semantic-diff reports.

The graph visualization is a bounded interactive sample backed by those
exhaustive lists and the embedded JSON. Select a node to focus its direct
changed-edge neighborhood and open a persistent inspector with its retained
kind, source path, changed-field names, incoming and outgoing relationships,
and related semantic findings. Inspector links open an exact source patch or
finding only when the report contains a matching target. Context-only endpoints
show their identifier and known relationships without implying unavailable
metadata. If JavaScript is disabled, the exhaustive lists remain the
authoritative fallback.

Finding prose resolves retained entity identities to human-readable symbol
names. This applies to subjects, dependency endpoints, affected consumers,
witness-path hops, evidence record keys, and semantic before/after values.
Raw stable IDs remain unchanged in JSON, alongside `entity_display_names`, so
automation and exact traceability are preserved.

After writing any HTML page, an interactive Compass CLI asks before opening it
in the default browser; Enter or `n` leaves the page closed. Scripts, pipes,
redirected commands, and CI never prompt or launch a browser.

## PR review JSON, Markdown, and SARIF

```bash
compass review --base BASE --head HEAD --format json
compass review --base BASE --head HEAD --format markdown
compass review --base BASE --head HEAD --format sarif
```

JSON uses strict schema `compass.pr_intelligence.report/1` and is the canonical
authority. It binds exact revision and graph-profile identity, evidence
manifest, completeness, ordered `cmpprv1` findings, rubric factors, advisory
risk, deterministic gates, canonical omissions, and a content digest.

Markdown and text show compact revision and finding references, plain-language
statuses, and relationship-only evidence-path summaries. They expose the same
finding count unless an explicit Markdown projection budget omits findings; in
that case the footer states the exact omitted count. The canonical report and
digest are unchanged. Finding statements and SARIF messages resolve retained
entity identities to human-readable names. Full revisions, fingerprints,
stable source/target identities, and witness endpoints remain available in
canonical JSON and SARIF for machine traceability.
SARIF 2.1.0 stores each Compass fingerprint in `partialFingerprints` and keeps
report identity, completeness, factors, gates, evidence, and omissions in
properties. SARIF severity is a presentation hint, not merge policy.

See the [PR Intelligence contract](pr-intelligence.md) before writing a
consumer.

## History export

### `graph-json`

```bash
compass history export REV \
  --format graph-json \
  --output graph.json
```

Reconstructs canonical graph JSON from a validated realization.

### `compass-out`

```bash
compass history export REV \
  --format compass-out \
  --output directory
```

Restores:

- authoritative non-derivable sidecars verbatim;
- graph artifacts;
- derived reports/HTML only using recorded compatible renderer versions.

## Equivalence

Semantic/canonical equivalence includes:

- same nodes and stable identities;
- same relationships and direction;
- same relevant attributes;
- same multiplicity;
- same duplicate id-less hyperedges;
- same authoritative bytes.

It does not require:

- same insignificant JSON object member order;
- same platform filesystem timestamp;
- same operational timing/token data;
- same derived byte order where the renderer contract allows semantic
  comparison.

## Binary caches

Query caches live under the graph output cache directory with versioned magic
and graph file signature. They are:

- acceleration only;
- bounded relative to source graph size;
- invalidated when signature/format changes;
- safely rebuildable.

Do not archive them as the only graph copy.

Versioned history uses a repository-private `cache/v1` directory below the Git
common directory. It contains verified-content extraction entries plus
canonical semantic-diff and viewer projections. This is a hard-cutover cache:
older layouts are ignored, not migrated. Everything below `cache/v1` is
reproducible from Git commits and immutable realizations.

## Other exports

`compass export` can produce:

- HTML and call-flow HTML;
- SVG;
- GraphML;
- Cypher;
- Obsidian/wiki/canvas-style documents;
- Neo4j/FalkorDB operations.

Each format has separate escaping, direction, multiplicity, and size concerns.
Use its command help and retain the source graph.

First-party editor and offline-viewer contracts are versioned independently:

- `compass.viewer.graph/1` — shared interactive graph model; located edges may
  include an optional `relationshipSite` source anchor, while exact Agent Graph
  views carry the pinned composition profile, bounded Retraction history,
  Challenge details, and Grounding metadata;
- `compass.graph-overview/1` — rebuildable prepared graph projection used by
  editor integrations;
- `compass.program.call_graph/1` — bounded symbol-centered caller/callee graph;
- `compass.viewer.architecture/1` — source-scoped subsystem architecture with
  typed relationships, hierarchy, omissions, and quality diagnostics;
- `compass.history.timeline/1` — commit and materialization states;
- `compass.history.change_counts/1` — lazy structural counts between existing
  realizations;
- `compass.graph-insights/1` — bounded structural-gap and disconnected-component
  evidence with witnesses and omission limits;
- `compass.graph-blind-spot-history/1` — active/resolved blind-spot trends over
  immutable history observations;
- `compass.orientation/2` — fitted Agent Orientation with optional typed
  `blindSpots` evidence;
- `compass.history.viewer_graph/1` — exact historical graph envelope;
- `compass.semantic_diff.report/1` — exhaustive semantic findings, source
  changes, and exact added, removed, and changed node/edge records consumed by
  the CLI HTML report and editor comparison views;
- `compass.ide.progress/1` — newline-delimited guided-operation events.
- `compass.agent-graph.overlay/1` — one immutable logical Overlay state;
- `compass.agent-graph.ingestion-preparation/1` — read-only, verifier-owned
  Base references, source evidence, and current expected revision for drafting
  a change batch;
- `compass.agent-graph.receipt/1` — atomic publication receipt;
- `compass.agent-graph.effective/1` — Base Graph plus one exact Overlay
  Revision and composition profile;
- `compass.agent-graph.rebase-plan/1` — exact, digest-bound rebase decision;
- `compass.agent-graph.audit/1` and `audit-result/1` — bounded operational
  attestations without prompts, credentials, chain-of-thought, or excerpts;
- `compass.agent-knowledge/1` — bounded task-context projection for one exact
  Effective Graph.

Agent Graph digests are lowercase SHA-256 and all contracts reject unknown
fields. `GROUNDED` appears only in Compass-produced results. Effective exports
carry the Base Generation, Overlay Revision, profile, composition version, and
effective identity; consumers must validate all of them before caching or
joining results. A profile change requires a newly composed Effective Graph;
viewer clients do not reinterpret one effective identity under another profile.

## Graph quality diagnostics

Use `compass diagnose quality --graph <path> --json` to inspect the typed graph
before giving it to an agent or downstream exporter. The report includes
evidence confidence, source-anchor coverage, external placeholders, dangling
relationships, publication omissions, identity collisions, and consistency
with the publisher statistics and overview sidecars.

For graphs larger than the default bounded in-memory reader cap, the command
returns `quality_scope: "publisher-stats-only"`: counts and omission metadata
come from `output-stats.json`, while record-level ratios are reported
as unavailable. This is an explicit safety boundary; use a prepared store or a
bounded investigation with `COMPASS_MAX_GRAPH_BYTES` rather than silently
allocating an unbounded JSON graph.

## Filesystem and concurrency

- Wait for the producing command to succeed.
- Avoid multiple writers to one output directory.
- Use distinct output paths for comparisons.
- Keep old output until new output validates when building critical
  integrations.
- Treat disk-full and permission errors as failed publication.
- Do not copy live history SQLite without its WAL state.

## Related pages

- [Graph model](../concepts/graph-model.md)
- [Integrating Compass](../guides/integrating-compass.md)
- [Versioned history](../guides/versioned-history.md)
- [Command reference](commands.md)

**Next step:** identify the most structured available output for your consumer
and validate its major version/direction/multiplicity before reading values.
