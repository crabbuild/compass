# Markdown graph-v1 intelligence hardening

Status: implementation and release qualification in progress

Public contract: `compass.graph/1`
Owner boundaries: `compass-languages`, `compass-graph`, `compass-model`,
`compass-query`, `compass-store`, and `compass-output`

## Objective

Improve Markdown graph usefulness without changing the graph schema or removing
table, header, row, or cell nodes. The implementation must make those nodes
semantically useful, stable, source-grounded, searchable, and harmless to
architecture topology. It must remain deterministic, bounded, local-first, and
fail closed.

## Compatibility decision

`compass.graph/1` remains the only published graph contract. Rich document facts
may exist while one file is normalized, but they are not a wire contract. The
graph publisher resolves their evidence and converts every document node to the
established graph-v1 `resource` details before validation and publication.
Validation rejects normalization-only document details if they reach a graph-v1
artifact.

Document roles are recovered centrally from extractor-owned qualified identity,
not display text and not a new serialized field. This keeps query, clustering,
analysis, and output behavior consistent while old strict readers continue to
receive the graph-v1 shape they already understand.

## Quality target

The reviewed external Markdown graph extractor is the comparative baseline. Its
public deterministic implementation recognizes ATX headings and document links,
skips fenced code, and publishes line-level locations. Compass must cover those
useful facts and additionally qualify:

- ATX and Setext headings with exact byte/line/column anchors;
- fenced code and the existing Markdown block vocabulary;
- table, header, row, and cell hierarchy;
- semantic table labels derived from headers and values;
- exact link/reference ownership by the smallest containing cell;
- conservative exact, ambiguous, unresolved, and limited resolution;
- stable identities across line shifts and non-identity cell edits;
- search parity between scan and immutable-index paths;
- isolation of table navigation containment from architecture topology; and
- explicit per-table limit evidence without suppressing later document facts.

The comparison is source-based. No external graph implementation is added as a
runtime, test, configuration, artifact, or fallback dependency.

## Phase 1: Contract boundary

Context: typed document facts previously risked becoming an accidental new
public schema.

Execution:

1. Keep publication and strict loading on `CODE_GRAPH_SCHEMA_V1`.
2. Reject normalization-only document details in the graph-v1 validator.
3. Resolve document references before converting document details to graph-v1
   resource details.
4. Remove graph-v2 adapters, gates, workflow targets, and migration claims.

Acceptance criteria:

- every newly published graph reports `compass.graph/1`;
- every document node uses graph-v1 resource details on the wire;
- unknown majors fail explicitly;
- strict graph-v1 load/round-trip tests pass; and
- stable IDs, edge direction, multiplicity, anchors, and provenance survive the
  normalization projection.

## Phase 2: Semantic table extraction

Context: generic `pipe_table_row` and `pipe_table_cell` labels provide syntax
volume but little retrieval or inspection value.

Execution:

1. Retain table, header, row, and cell nodes.
2. Give each node a section-qualified, occurrence-safe identity.
3. Label tables with section/header context, rows as `Header=value`, and cells
   as `Header: value`, including explicit empty/limited labels.
4. Preserve exact source anchors and the full containment hierarchy.
5. Assign inline links and backtick code references to the smallest exact cell.

Acceptance criteria:

- all four table roles are present for a normal pipe table;
- labels are meaningful without consulting private attributes;
- nested anchors are contained by their parent anchors;
- row and cell IDs survive unrelated line insertion and non-identity edits;
- references originate at the containing cell; and
- output is byte-deterministic for equivalent input.

## Phase 3: Bounds and failure truthfulness

Context: a giant early table must not consume the global block budget and hide
later headings.

Execution:

1. Enforce independent table caps of 20,000 structural nodes, 16,384 cells, and
   512 KiB retained text.
2. Count omitted facts and emit bounded diagnostics.
3. Continue scanning the document after the table budget is exhausted.

Acceptance criteria:

- no table exceeds any configured cap;
- a limit is not reported as an empty table;
- omitted counts are deterministic and truthful;
- source anchors remain ordered and in bounds; and
- headings after an oversized table are still extracted.

## Phase 4: Retrieval and topology

Context: semantic table content must be discoverable, but navigation
containment must not inflate architecture centrality.

Execution:

1. Index semantic node names and qualified identities in both scan and
   immutable snapshot paths.
2. Keep exact document-to-code and document-to-file references as ordinary
   graph-v1 reference edges.
3. Exclude containment edges touching table navigation nodes from architecture
   degree, clustering, and topology summaries only.
4. Keep those nodes and edges available for search, traversal, inspection, and
   source navigation.

Acceptance criteria:

- scan and immutable-index rankers retrieve the same table cell query;
- exact references resolve to their unique code/file targets;
- ambiguous and unresolved references never acquire invented targets;
- topology scores do not change merely because a table gains cells; and
- table nodes remain present in the graph and viewer.

## Phase 5: Independent qualification

Context: parser snapshots alone can reproduce extractor mistakes. Quality needs
a source-derived oracle.

Execution:

1. Add an adversarial Markdown fixture with tables and exact local/code links.
2. Run an independent source oracle against the published graph-v1 artifact.
3. Integrate it into `qualify_code_graph_v1.sh --fixtures-only`.
4. Verify the product boundary so no Graphify dependency enters Compass.

Acceptance criteria:

- the oracle verifies schema integrity, role counts, semantic labels,
  hierarchy, anchors, reference ownership, and exact targets;
- repeated qualification builds are byte-identical;
- the quality score meets the checked-in threshold;
- the graph-v1 fixture release gate passes; and
- `scripts/check_product_boundary.sh` passes.

## Rollback

Revert the extractor semantic-label/identity logic, graph-v1 normalization
projection, central role inference, topology filtering, and the independent
oracle together. Do not change the schema string, rewrite history, or remove
published table/header/row/cell records during rollback.
