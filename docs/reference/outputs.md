# Output reference

Compass outputs range from the current `compass-out/` directory to versioned
CompassQL results and immutable history exports. This reference describes
consumer responsibilities and authority.

> **Who this reference is for:** integrators, operators, and contributors
> changing an output or renderer.
>
> **You will learn:** core artifacts, graph JSON, query schemas, derived views,
> history export, caches, atomicity, and deterministic equivalence.
>
> **Prerequisites:** [Graph model](../concepts/graph-model.md).
>
> **Reading time:** 10–12 minutes.

## Current output directory

Default:

```text
compass-out/
├── graph.json
├── program.json
├── GRAPH_REPORT.md
├── graph.html
├── manifest.json
├── cache/
└── optional sidecars and exports
```

`--out DIR` or compatible `COMPASS_OUT` use can select another root.

## Authority table

| Artifact | Authority | Consumer use |
| --- | --- | --- |
| `graph.json` | machine-readable graph snapshot | queries, integrations, export |
| `program.json` | provenance-aware Program IR | program inspection, semantic analysis |
| `GRAPH_REPORT.md` | derived human orientation | architecture survey |
| `graph.html` | derived optional visualization | interactive exploration |
| `manifest.json` | incremental build state | next compatible update |
| binary query caches | disposable acceleration | internal query loading |
| semantic sidecars | depends on artifact class | completeness/evidence/export |

Do not reconstruct graph truth from HTML when JSON is available.

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

The private `.compass_output_stats.json` and sealed build state retain the same
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

## `graph.html`

Optional interactive visualization. It may be absent when:

- `--no-viz` was used;
- graph size exceeds a rendering limit;
- a specific build/export omitted it.

It is not required for query commands.

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

`program.json` is the canonical, language-neutral Program IR produced by native
`update`, `extract`, and `watch` builds. Its public schema identifier is:

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

- `compass.viewer.graph/1` — shared interactive graph model;
- `compass.program.call_graph/1` — bounded symbol-centered caller/callee graph;
- `compass.viewer.callflow/1` — broader subsystem architecture flow;
- `compass.history.timeline/1` — commit and materialization states;
- `compass.history.change_counts/1` — lazy structural counts between existing
  realizations;
- `compass.history.viewer_graph/1` — exact historical graph envelope;
- `compass.ide.progress/1` — newline-delimited guided-operation events.

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
- [Storage and history](../design/storage-and-history.md)
- [Command reference](commands.md)

**Next step:** identify the most structured available output for your consumer
and validate its major version/direction/multiplicity before reading values.
