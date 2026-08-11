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
├── manifest.json
├── program.json                 # only with --program or --program-artifact
├── graph-overview.json          # clustered builds
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
- suggested questions.

It is intended for people and can evolve in prose/format. Do not parse it when
structured data or command JSON exists.

Community evidence labels use the highest-connectivity member's concise name
when it is unique. When multiple communities share that name, Compass adds a
compact source or wiring-site anchor and, only if needed, the graph-local
community ID. These labels are deterministic navigation aids, not community
identity; consumers that need identity should use the community ID and member
set instead.

The report begins with a bounded Agent Orientation for first-session or broad
repository context. `orientation.json` is the versioned machine form of that
same fitted model. Compass publishes both from one coherent build input and
validates the graph generation and exact streamed `graph.json` digest before
`compass export orientation-json` or
`compass://orientation` returns it. `compass://report` renders the human report
from that validated model; it never trusts an adjacent Markdown file by name.

## `graph.html`

Optional interactive visualization. It may be absent when:

- `--no-viz` was used;
- graph size exceeds a rendering limit;
- a specific build/export omitted it.

It is not required for query commands.

The document is self-contained and uses the same versioned graph workbench as
the VS Code extension. It performs no runtime network requests, follows the
operating system's light or dark color scheme, and retains keyboard, reduced
motion, narrow-screen, and high-contrast behavior from the shared viewer.

When the node limit selects a community overview, the standalone document
embeds a deterministic bounded set of complete community details: at most
5,000 detail nodes and 40,000 internal detail edges across the export. Details
are validated only when opened. Double-click an available community node (or
use **Open community** in the inspector) to enter its member graph; use
**Overview** to return. Communities outside the embedded budget remain visible
and are marked as unavailable for standalone drilldown; use the VS Code graph
or `compass export json --community ID` to inspect one without loading every
community into the HTML page. Embedded details preserve internal edges, source
anchors, and hyperedges, while cross-community edges remain represented only
in the overview.

Large community overviews use a deterministic hub-centered layout. Physics is
paused, labels remain bounded, and at most 4,000 aggregate edges are rendered
as straight hairlines. The visible edges form a deterministic strongest-edge
backbone; the inspector continues to report the complete relationship count
and discloses the rendered count. This keeps repositories with thousands of
communities from producing an expensive rectangular edge curtain without
changing `graph.json` or the complete overview model.

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
by native `init`, `update`, `extract`, and `watch` builds when `--program` or
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
  include an optional `relationshipSite` source anchor;
- `compass.graph-overview/2` — rebuildable prepared graph projection used by
  editor integrations;
- `compass.program.call_graph/1` — bounded symbol-centered caller/callee graph;
- `compass.viewer.callflow/1` — broader subsystem architecture flow;
- `compass.history.timeline/1` — commit and materialization states;
- `compass.history.change_counts/1` — lazy structural counts between existing
  realizations;
- `compass.history.viewer_graph/1` — exact historical graph envelope;
- `compass.semantic_diff.report/1` — exhaustive semantic findings, source
  changes, and exact added, removed, and changed node/edge records consumed by
  the CLI HTML report and editor comparison views;
- `compass.ide.progress/1` — newline-delimited guided-operation events.

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
