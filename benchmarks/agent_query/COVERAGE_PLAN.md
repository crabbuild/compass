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
Cobra graphs, confirming all four named tools. This is interface availability
evidence; the cross-language community/hub questions still need execution.
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
