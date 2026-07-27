# VS Code Community Diff Detail Design

**Date:** 2026-07-26  
**Status:** Implemented and verified

## Problem

The Codebase Evolution changed-graph view compares overview projections. For a
large graph, each overview node represents a community rather than an individual
symbol. A changed community therefore appears as a single `Changed` node with
its current member count.

Opening that community currently loads only the selected revision. The
comparison context is lost, so developers cannot discover which symbols or
relationships were added, removed, or changed. A changed symbol also carries
only a status badge; the inspector does not explain which stored fields differ
between revisions.

## Goals

The enhanced comparison must let a developer:

1. move from a changed aggregate community to its exact symbol-level delta;
2. browse and search the affected symbols without relying on the graph layout;
3. distinguish added, removed, changed, and contextual symbols and
   relationships;
4. inspect the exact before and after values for every changed symbol field;
5. open the appropriate historical source location from either revision;
6. return to the aggregate comparison without recomputing it; and
7. retain the lazy-loading and bounded-memory behavior of the existing history
   view.

## Chosen Approach

Compass will lazily load the selected community from both compared revisions.
The extension host validates both historical identities and sends the two
community graph projections to the webview. The webview computes a focused
community comparison and renders it through the existing comparison-aware graph
workspace.

This approach is preferred over loading a repository-wide exact symbol diff
upfront because it preserves fast aggregate comparison startup and bounded
community detail. It is preferred over tooltip-only enrichment because it
exposes the evidence instead of merely adding another summary.

## Interaction Design

### Aggregate comparison

The existing changed-community node remains the lightweight entry point.
Selecting it pins the ordinary inspector, which adds a prominent
`Inspect changes` action for aggregate nodes in comparison mode. The action
describes the community as a collection of symbols, not as a count of changed
symbols, because the existing `memberCount` is the current membership total.

Activating the action, or using the existing community activation gesture,
starts the two-revision community request. The aggregate graph remains visible
while the request is running.

### Community comparison

When both community projections arrive, the graph frame shows a focused
symbol-level delta:

- added symbols and relationships;
- removed symbols and relationships;
- changed symbols and relationships; and
- unchanged endpoint symbols needed to explain changed relationships.

A breadcrumb-style Back action returns to the aggregate changed graph. A
summary states the exact visible counts for symbol and relationship changes.
The existing Added, Removed, Changed, and Context filters remain available.

The inspector makes `Changed symbols` the primary comparison section. It
provides a searchable, keyboard-accessible list ordered by change type and then
symbol label. Choosing a result focuses the matching graph node. The graph
remains useful for topology, while the list provides a deterministic way to
find the evidence in dense communities.

### Symbol evidence

Selecting a changed symbol shows a `What changed` table. It contains only fields
whose stored values differ, with `Before` and `After` columns. The comparison
supports at least:

- label;
- symbol kind;
- signature;
- language;
- community identifier and name;
- source file;
- source line or byte range;
- degree and member count when present;
- learning status metadata; and
- additional retained graph-node fields that differ.

Presentation-only metadata such as viewer color, the computed change status,
and attached diff evidence is excluded.

Missing values are rendered as `Not recorded`. Structured values are rendered
as bounded, readable JSON rather than as `[object Object]`. If an unknown field
contains an unusually large value, the inspector truncates its display while
retaining an accessible indication that the value was shortened.

Added and removed symbols show the complete available metadata from their
owning revision instead of an artificial two-column comparison.

### Relationship evidence

The selected-symbol inspector receives the connected edges rather than only a
deduplicated neighbor list. Each relationship row shows:

- relation type;
- the other endpoint;
- Added, Removed, Changed, or Context status; and
- confidence when recorded.

A changed relationship can expose its differing before and after fields using
the same bounded field-difference representation. This keeps topology evidence
separate from symbol-property evidence while making both discoverable from the
selected symbol.

### Source navigation

Source actions carry the revision that owns the displayed location:

- added symbols open the current revision;
- removed symbols open the parent revision; and
- changed symbols offer `Open before` and `Open after` when those locations are
  recorded.

The existing source-navigation safety checks remain authoritative. A missing or
non-navigable source location remains readable metadata but does not render an
enabled action.

## Data Model

The comparison result will retain immutable evidence in addition to the
presentation record:

```ts
type GraphFieldChange = {
  field: string;
  before: unknown;
  after: unknown;
};

type GraphRecordEvidence<T> = {
  before?: T;
  after?: T;
  fields: GraphFieldChange[];
};
```

Graph nodes and edges in a comparison may carry record evidence. The evidence
snapshots exclude computed comparison properties to avoid recursive data and
false differences.

The comparison function will use a canonical comparison projection rather than
raw `JSON.stringify` ordering. It excludes presentation-only keys, compares
nested source locations deterministically, and produces field differences from
the same projection used to assign `Changed`. This guarantees that every
`Changed` badge has explainable evidence.

The graph contract remains backward compatible: evidence fields are optional,
and ordinary graph projections do not include them.

## Extension and Webview Protocol

The webview adds a dedicated community-comparison request containing:

- request identifier;
- current commit and expected realization/fingerprint;
- parent commit and expected realization/fingerprint; and
- selected community identifier.

The extension host:

1. validates that the request belongs to the active comparison;
2. loads both community projections in parallel through `RevisionStore`;
3. verifies each projection against its expected historical identity; and
4. returns both projections in one correlated response.

The response includes both commits and the request identifier. The webview
accepts it only when the selected commit, comparison parent, community, and
active request still match. Changing commits, leaving comparison mode, opening
another community, or returning to the overview invalidates the previous
request.

Ordinary single-revision `openCommunity` messages remain unchanged.

## State and Data Flow

```text
aggregate changed community
  -> request current + parent community detail
  -> RevisionStore loads both projections in parallel
  -> extension validates both immutable identities
  -> webview rejects stale or mismatched responses
  -> compareGraphs retains before/after record evidence
  -> focused community delta graph
  -> searchable changed-symbol list
  -> selected symbol field and relationship evidence
```

The aggregate comparison remains in webview memory while detail is open, so
Back is immediate. Community exports continue to use the configured graph node
limit and the existing bounded LRU caches.

## Loading, Errors, and Limits

During loading, the aggregate graph stays mounted and reports which community
is being compared. Duplicate activation is ignored while that request is
active.

If either community cannot be loaded, the user remains on the aggregate
comparison. The inspector shows a concise error and a retry action. Identity
mismatch errors instruct the user to refresh the revision comparison.

Compass detects a bounded community by comparing the loaded detail counts with
the member counts retained on the parent and current aggregate community nodes.
If either side cannot contain its complete aggregate membership under the
configured node limit, Compass must not imply that the visible delta is
complete. The detail view displays a persistent bounded-results notice
containing the limit and recommends increasing `compass.graphNodeLimit` when
the complete community is required. Counts are described as visible counts
while that notice is present.

An absent community on one side is valid. It is represented by an empty
projection, making all symbols on the other side added or removed. A missing
community caused by an invalid export is an error, not an empty projection.

## Accessibility

Change state continues to use text in addition to color. The changed-symbol
list uses buttons with visible labels and status text. The Before/After table
has explicit column headers, and missing or truncated values have readable
labels.

Loading and error messages use status and alert semantics. Keyboard users can
search, select a changed symbol, inspect its fields and relationships, use
source actions, and return to the aggregate comparison without interacting with
the canvas.

## Verification

Unit and component tests will cover:

- deterministic field comparison and exclusion of presentation metadata;
- before/after evidence for changed nodes and edges;
- complete metadata behavior for added and removed symbols;
- exact community-level node and relationship counts;
- empty community handling on either side;
- community-comparison request and response validation;
- parallel current/parent loading and historical identity checks;
- stale response rejection after commit, parent, community, or mode changes;
- changed-symbol search, ordering, focus, and keyboard behavior;
- Before/After rendering for missing, nested, and bounded values;
- relationship status and field evidence;
- parent/current source-revision selection;
- loading, retryable error, and bounded-results states;
- accessibility labels and status semantics; and
- unchanged behavior in ordinary and aggregate graph views.

Browser coverage will verify the end-to-end drill-down from a changed aggregate
community into a symbol-level delta, field inspection, source actions, and Back
navigation. Existing viewer, VS Code build, and VSIX smoke checks remain part of
the completion gate.

After code changes, `graphify update .` refreshes the project knowledge graph.

## Scope Boundaries

This work does not add source editing, patch application, diff comments,
cross-community identity remapping, repository-wide eager symbol loading, or a
new history storage format. It reuses existing historical community exports and
adds comparison evidence and interaction in the VS Code extension and shared
viewer.
