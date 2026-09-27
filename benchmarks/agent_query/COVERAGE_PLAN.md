# Real-repository comparison coverage

This plan extends the existing paired runs to the user's requested query,
ask, node, path, caller/callee, cluster/community, and god-node surfaces.
It is a work plan, not a completed evaluation or a superiority claim.

## Repository panel

Keep the same clean, pinned source on both sides. The current panel includes
Cobra (Go), Flask (Python), Gson (Java), Zod (TypeScript), and Axum (Rust),
plus the separate fd (Rust) source-first sample. Their commits and source roots
are recorded in `suite_v2.toml`, `suite_fd.toml`, and each captured run.
Use additional independently selected repositories for confirmation after
improving on these development cases. Do not relabel used questions as held out.

## Question and evidence matrix

Each new question must record its exact source witness, expected outcome,
per-tool operation, bounds, and judgment before executing that question.
No output-derived oracle correction may silently replace an earlier result.

| Surface | Questions on every language | Primary correctness evidence |
| --- | --- | --- |
| Node lookup | Locate an exact declaration; distinguish same-named declarations; reject an absent symbol | Exact file, declaration start, terminal name, and owner; ambiguity is not a successful guess |
| Explain | Explain the declaration's role, source, incoming and outgoing relationships | Source-reviewed statements and exact endpoint identities; score unsupported extra claims as well as omissions |
| Callers | Identify direct callers, repeated call sites, and a same-named wrong-owner negative | Source call occurrences, caller-to-callee direction, multiplicity, and provenance |
| Callees | Identify direct targets, including constructors, loop receivers, and callbacks | Complete reviewed local target set for the selected function; unresolved dynamic targets stay explicit |
| Native path/walk | Find a directed call path and an undirected navigation path; handle ambiguity, disconnection, and bounds | Ordered adjacent edges, directions, source-supported occurrences, and explicit incomplete/limit outcomes |
| Natural query | Answer a source-level task with a fixed answer budget and permitted continuations | Reviewed relevant facts, unsupported returned facts, and full workflow cost |
| Ask/routing | Ask who calls a declaration, what it calls, and how two declarations connect | Same semantic witnesses as direct operations; additionally check requested direction and operation selection |
| Community lookup | Identify a selected node's community and enumerate its bounded membership | Exact identities and membership in that tool's captured graph; this is graph consistency, not source correctness |
| Cluster summary | Describe major functional groups and the connections between them | Separately reviewed source responsibilities and boundary relations; arbitrary cluster IDs/names are never gold |
| God nodes/hubs | Return top-N connected source symbols and explain their links | Independently recomputed degree/eligibility on the captured graph, plus source review of returned declarations/edges |
| Design diagnosis | Assess whether a candidate mixes responsibilities or merely has many users | Explicit source-based judgments and counterexamples; degree alone cannot establish a god-object defect |

## Equivalent operations

The installed versions are inspected through their own help before selecting
commands. The existing suite pairs `explain`, `path`, natural `query`,
`callers`/`affected`, and `callees`/`explain`. Both sides use the same relation
direction and comparable depth/budget wherever both expose those controls.

Compass exposes natural `ask`, typed `search`, `architecture`, and the MCP
tools `get_neighbors`, `get_community`, `god_nodes`, and `graph_stats`.
Graphify 0.9.67 exposes CLI `god-nodes --top N --json`; its inspected CLI help
does not advertise dedicated `ask` or community-membership commands.
Further installed-source inspection and `python -m graphify.serve --help`
confirm a public MCP server in `graphify.serve`, including `get_neighbors`,
`get_community`, `god_nodes`, and `graph_stats`. These can be compared directly
with the matching Compass MCP tools. The currently pinned Graphify environment
lacks the optional `mcp` SDK. An isolated `graphifyy[mcp]==0.9.67` environment
now provides it; all 227 compared Graphify package files match the original
installation. The environment manifest pins its separate dependencies. Actual
MCP initialize/tools-list handshakes succeeded for both tools on the retained
Cobra graphs, confirming all four named tools. The subsequent `suite_mcp.json` runs exercise these interfaces across all five
languages. Their graph-consistency results are recorded in the audit report;
source-level cluster quality and god-object design judgments remain pending.
Graphify community lookup accepts an
explicit token budget; account for that bound and any truncation separately.
Lack of one CLI command name does not establish lack of the capability.

Where the available workflows use different interfaces, retain all requests,
startup/setup work, output bytes, follow-ups, and errors. Report protocol
overhead separately from answer content. A developer adapter may invoke a
documented public operation; it must not synthesize an answer by reading the
graph on behalf of only one tool. Independent graph inspection belongs to the
oracle, not the measured answer path.

Native-only structural runs use no model-generated edges or community labels
on either side. A future model-assisted comparison requires a separate arm
with the same model, credentials policy, budget, and complete cost accounting.
Do not compare one tool's model-assisted output with the other's native output.

## Scoring and acceptance

### Hub explanation and source-role review

The post-output `hub_role_reviews.json` census covers all original fifty hub
entries per tool. Manually assign declaration/container roles only for exact
returned identities or globally unique displayed labels. Preserve unidentified
entries as unknown; do not choose the candidate that fits its degree. Verify
the pinned source commit, whole-file hash, exact anchor, and excerpt. These are
source-role descriptions, not god-object labels or a representative precision
sample, and each tool returns a different set.

For the connectivity improvement, independently recompute each returned hub's
incident record total, self-loops, and bounded per-relation direction counts.
Distinguish these from distinct-pair ranking degree. Keep undirected artifacts
undirected. Check both structured values and the matching text block. An absent
summary is unavailable in that response, not an incorrect answer; neither a
neighbor follow-up workflow nor Graphify's separate CLI is excluded by this
finding. Do not turn summary availability into a cross-tool accuracy score.

### MCP path diagnostics

`suite_mcp_paths.json` covers prepared exact-ID navigation, reverse traversal,
hop cutoffs, missing/ambiguous endpoints, and disconnected pairs. The initial
capture occurred before its planned registration commit; its archived
preregistration claim is superseded by the development designation in the
audit report. `suite_mcp_path_labels.json` is a separate post-diagnostic arm
using the same positive endpoints and bounds with each tool's display labels.
Graphify's public path input description advertises labels/keywords, so the
label arm is necessary context for the ID failures and must be reported.

Both arms use undirected navigation with identical external bounds. Audit
actual ordered identities, relations, direction, minimum distance, and source
anchors; distinguish topology consistency from reviewed source-route evidence.
Do not substitute expected IDs for ambiguous returned labels. A depth-limited
miss remains incomplete for a global-disconnection question. These development
arms do not complete directed call-flow, representative edge precision, path
occurrence recall, or held-out confirmation requirements.

### Hub navigation diagnostic

The label-identity gap found in the first MCP run motivates a separate
development diagnostic. Preserve that run and its label-only results. For
each of the ten hubs returned on each of the five repositories, issue one
`get_neighbors` follow-up using only the returned exact ID when available,
otherwise its returned label. Do not read a graph to substitute an ID for
either product. Both products get the same one-follow-up allowance and the
existing full-enumeration bounds. Explicit ambiguity is safe but does not
complete direct navigation. This diagnostic does not measure workflows with
additional disambiguation steps or Graphify's separate CLI JSON hub workflow.

The independent oracle may use the captured graph to verify exact identity,
degree, source anchors, and the multiset of displayed direction/neighbor-label
pairs after grouping by distinct neighbor. It must not resolve identity using
the expected degree. Unfiltered neighbors may select one relation per neighbor;
this diagnostic does not score complete parallel-relation or occurrence recall.
Capture every response, failure, and output byte. Source-based design quality,
complete top-N eligibility, and ranking correctness remain separate questions.

- Publish category-level results and every failure, including competitor wins.
- Keep source correctness, graph consistency, task availability, and output
  efficiency separate. No single combined score may hide a precision failure.
- A text anchor pass does not certify all statements in an answer. Add source
  review of returned edges/owners before making precision claims.
- Score community partition metrics separately from functional usefulness.
  Different valid partitions are possible; matching a package directory is at
  most a structural proxy, not proof of cohesion or architectural quality.
- Compare tokens on the same successful questions and retain total continuation
  cost. Bytes/4 estimates must stay labeled as estimates.
- Repeated release-build timings on quiet, matched inputs are required for
  speed claims. Concurrent debug runs provide correctness evidence only.
- Existing fixes and development-suite leads do not satisfy the complete
  objective. God-object judgments and the additional community/ask surfaces
  still require measured evidence across the repository panel.
