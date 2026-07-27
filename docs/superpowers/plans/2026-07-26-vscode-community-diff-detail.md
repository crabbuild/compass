# VS Code Community Diff Detail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let developers drill from an aggregate changed community into an exact symbol/relationship delta and inspect every changed field before and after.

**Architecture:** Keep the initial history comparison aggregate and fast. On community activation, the VS Code host loads the parent and current historical community projections in parallel, the webview rejects stale responses and computes a detailed comparison, and shared viewer components render searchable change evidence. Comparison records retain canonical before/after snapshots and field changes while ordinary graphs remain backward compatible.

**Tech Stack:** TypeScript 5.9, React 19, Zod 4, VS Code Webview API, Vitest, Testing Library, Playwright, vis-network.

## Global Constraints

- Follow the user's requested implementation-first sequence: implement each behavior before adding its regression tests; do not use the red/green TDD loop.
- Preserve ordinary current-graph and single-revision history community behavior.
- Keep historical detail lazy and bounded by `compass.graphNodeLimit`, default `5000`.
- Exclude `color`, `change`, and `evidence` from stored-field comparisons.
- Treat all graph and webview payloads as untrusted and validate them through the existing Zod contracts.
- Ignore late responses when commit, parent, community, request identifier, or comparison mode no longer matches.
- Run `graphify update .` after code changes.

---

### Task 1: Explainable graph-record comparisons

**Files:**
- Modify: `packages/compass-viewer/src/contracts/graph.ts`
- Create: `packages/compass-viewer/src/history/recordDiff.ts`
- Modify: `packages/compass-viewer/src/history/ComparisonOverlay.tsx`
- Modify: `packages/compass-viewer/src/history/ComparisonOverlay.test.tsx`

**Interfaces:**
- Produces:
  - `GraphFieldChange = { field: string; before?: unknown; after?: unknown }`
  - `GraphRecordEvidence = { before?: Record<string, unknown>; after?: Record<string, unknown>; fields: GraphFieldChange[] }`
  - optional `evidence` on `GraphNode` and `GraphEdge`
  - `compareRecord(before, after): GraphRecordEvidence`
  - `compareGraphs(parent, current): GraphComparison` with retained evidence and aggregate-mode preservation

- [ ] **Step 1: Add backward-compatible graph evidence contracts**

Add Zod schemas and inferred types in `contracts/graph.ts`:

```ts
export const GraphFieldChangeSchema = z.object({
  field: z.string().min(1),
  before: z.unknown().optional(),
  after: z.unknown().optional()
});
export const GraphRecordEvidenceSchema = z.object({
  before: z.record(z.string(), z.unknown()).optional(),
  after: z.record(z.string(), z.unknown()).optional(),
  fields: z.array(GraphFieldChangeSchema)
});
```

Attach `evidence: GraphRecordEvidenceSchema.optional()` to both graph record
schemas and export their inferred types.

- [ ] **Step 2: Implement canonical record comparison**

Create `recordDiff.ts` with:

```ts
const PRESENTATION_FIELDS = new Set(["color", "change", "evidence"]);
export function compareRecord(
  before: Record<string, unknown> | undefined,
  after: Record<string, unknown> | undefined
): GraphRecordEvidence;
export function displayFieldValue(value: unknown, maxLength = 240): {
  text: string;
  truncated: boolean;
};
```

Recursively sort object keys, omit presentation fields at every record root,
flatten nested object changes using dotted paths such as `source.startLine`,
treat arrays as atomic values, preserve explicit `undefined` as an absent side,
and sort field changes lexically.

- [ ] **Step 3: Retain evidence in graph comparisons**

Replace `JSON.stringify` equality in `compareGraphs` with `compareRecord`.
For a changed node or edge, attach both canonical snapshots and the non-empty
field list. Added and removed records retain their owning record plus their
status. Set `stats.aggregated` to
`parent.stats.aggregated || current.stats.aggregated`; detailed community
inputs remain non-aggregated.

- [ ] **Step 4: Add regression coverage after implementation**

Extend `ComparisonOverlay.test.tsx` with cases that assert:

```ts
expect(changedNode.evidence?.fields).toEqual([
  { field: "signature", before: "old()", after: "new()" },
  { field: "source.startLine", before: 2, after: 4 }
]);
expect(changedNode.evidence?.fields.map((field) => field.field))
  .not.toContain("color.background");
expect(comparison.graph.stats.aggregated).toBe(true);
```

Also cover key-order independence, nested missing values, changed edge
evidence, and bounded structured display.

- [ ] **Step 5: Verify Task 1**

Run:

```bash
npm test -w @compass/viewer -- ComparisonOverlay
npm run typecheck -w @compass/viewer
```

Expected: all targeted tests pass and TypeScript reports no errors.

---

### Task 2: Detailed symbol and relationship inspector

**Files:**
- Create: `packages/compass-viewer/src/graph/ChangeEvidence.tsx`
- Create: `packages/compass-viewer/src/graph/ChangedSymbolList.tsx`
- Modify: `packages/compass-viewer/src/graph/GraphInspector.tsx`
- Modify: `packages/compass-viewer/src/graph/CompassGraph.tsx`
- Modify: `packages/compass-viewer/src/graph/NodeHoverCard.tsx`
- Modify: `packages/compass-viewer/src/theme.css`
- Create: `packages/compass-viewer/src/graph/ChangeEvidence.test.tsx`

**Interfaces:**
- Consumes: optional node/edge `evidence` from Task 1.
- Produces:
  - `GraphSourceRevisions = { before: string; after: string }`
  - `GraphHost.openSource(source, revision?)`
  - optional `communityDetail.bounded` notice metadata
  - comparison inspector sections for affected symbols and connected edges

- [ ] **Step 1: Implement changed-symbol discovery**

Create `ChangedSymbolList` that receives:

```ts
{
  nodes: GraphNode[];
  query: string;
  selectedId?: string;
  onFocus(nodeId: string): void;
}
```

Include nodes whose change is `added`, `removed`, or `changed`; filter by label,
kind, and source path; sort with `changed`, `added`, `removed` groups then
locale-aware label order; render status text and a maximum initial window of
100 rows with a Show all control.

- [ ] **Step 2: Implement before/after evidence**

Create `ChangeEvidence` that receives the selected node, all connected edges,
a node lookup, source revisions, and callbacks. Render:

- a `What changed` table for `node.evidence.fields`;
- complete ordinary metadata for added/removed nodes through the existing
  inspector content;
- `Open before` and `Open after` actions derived from
  `evidence.before.source` and `evidence.after.source`;
- relationship rows containing relation, endpoint, confidence, and status;
- a nested field table for changed-edge evidence.

Use `displayFieldValue` for every cell, `Not recorded` for absent values, and a
visible/accessible truncated marker.

- [ ] **Step 3: Wire comparison detail into the graph workspace**

Change the graph host signature to:

```ts
openSource(source: SourceLocation, revision?: string): void;
```

Pass connected `GraphEdge[]`, revisions, and evidence callbacks into
`GraphInspector`. In aggregate comparison mode, label the community action
`Inspect changes` with `${memberCount} current symbols`; outside comparison
mode retain `Open community`.

Preserve aggregate-mode activation so double-clicking a changed community calls
`openCommunity`. Add an optional bounded notice:

```ts
bounded?: { limit: number; parentMembers: number; currentMembers: number };
```

The notice must say the visible comparison may be incomplete and name
`compass.graphNodeLimit`.

- [ ] **Step 4: Style the evidence without widening the inspector**

Add VS Code-token-based styles for the symbol list, Before/After table,
relationship list, truncation label, bounded notice, focus/hover states, narrow
inspector wrapping, and high-contrast borders. Do not introduce fixed light
background colors.

- [ ] **Step 5: Add regression coverage after implementation**

Test changed-field headers and cells, missing values, truncation text,
relationship status/confidence, changed-edge fields, changed-symbol sorting and
filtering, revision-aware source callbacks, aggregate `Inspect changes` copy,
and unchanged ordinary graph copy.

- [ ] **Step 6: Verify Task 2**

Run:

```bash
npm test -w @compass/viewer -- ChangeEvidence GraphInspector CompassGraph
npm run typecheck -w @compass/viewer
```

Expected: all targeted tests pass and TypeScript reports no errors.

---

### Task 3: Dual-revision community protocol and historical source

**Files:**
- Modify: `editors/vscode/src/history/panelMessages.ts`
- Modify: `editors/vscode/src/history/panelMessages.test.ts`
- Modify: `editors/vscode/src/history/revisionStore.ts`
- Modify: `editors/vscode/src/history/revisionStore.test.ts`
- Create: `editors/vscode/src/history/historicalSource.ts`
- Create: `editors/vscode/src/history/historicalSource.test.ts`
- Modify: `editors/vscode/src/views/historyPanel.ts`

**Interfaces:**
- Produces:
  - `compareCommunity` webview request containing request/commit/parent,
    both identities, community ID, and side-presence flags
  - `communityComparison` response containing optional current/parent graphs
    and `nodeLimit`
  - `communityComparisonError` correlated error response
  - `HistoricalSourceProvider.open(commit, source)`

- [ ] **Step 1: Add protocol unions and operation mapping**

Define the request:

```ts
{
  type: "compareCommunity";
  requestId: string;
  commit: string;
  parent: string;
  currentIdentity: { realization: string; fingerprint: string };
  parentIdentity: { realization: string; fingerprint: string };
  communityId: number;
  hasCurrent: boolean;
  hasParent: boolean;
}
```

Define success/error host messages with matching correlation fields. Add
`"Compare community"` to `HistoryOperation` and map the request in
`historyOperationFor`.

- [ ] **Step 2: Load both community projections in parallel**

In `historyPanel.ts`, validate both commits against the loaded timeline and both
identities against `historyIdentity(entry)`. Require at least one side. Call
`RevisionStore.loadCommunity` only for present sides:

```ts
const [current, parent] = await Promise.all([
  hasCurrent ? revisions.loadCommunity(commit, communityId, graphNodeLimit, currentIdentity) : undefined,
  hasParent ? revisions.loadCommunity(parent, communityId, graphNodeLimit, parentIdentity) : undefined
]);
```

Post one correlated success response; route errors to
`communityComparisonError` without replacing the aggregate comparison.

- [ ] **Step 3: Implement exact historical source documents**

Create a panel-scoped `HistoricalSourceProvider` using a unique URI scheme.
Validate that source paths remain lexically inside the repository, run:

```bash
git show --no-textconv <commit>:<repo-relative-path>
```

with `execFile` argument arrays, an 8 MiB output bound, and no shell. Use the
URI path to preserve the original file extension for VS Code language
detection. Reveal the recorded line/byte range after opening. Only allow the
active comparison's current or parent commit. Dispose the provider with the
history panel.

- [ ] **Step 4: Add protocol and host regression coverage after implementation**

Cover union acceptance, operation mapping, parallel load cache keys, identity
mismatch, absent-side behavior, correlated errors, path traversal rejection,
git argument construction, source range selection, and provider disposal.

- [ ] **Step 5: Verify Task 3**

Run:

```bash
npm test -w editors/vscode -- panelMessages revisionStore historicalSource
npm run typecheck -w editors/vscode
```

Expected: all targeted tests pass and TypeScript reports no errors.

---

### Task 4: History webview state and lazy drill-down

**Files:**
- Modify: `editors/vscode/src/webviews/history.tsx`
- Modify: `packages/compass-viewer/src/history/HistoryWorkspace.tsx`
- Modify: `packages/compass-viewer/src/history/HistoryWorkspace.test.tsx`

**Interfaces:**
- Consumes: Task 1 `compareGraphs`, Task 2 detail/revision props, Task 3 protocol.
- Produces: stale-safe community comparison state and immediate Back behavior.

- [ ] **Step 1: Retain both comparison identities**

On a `comparison` host message, store:

```ts
{
  current: { realization: message.realization, fingerprint: message.fingerprint },
  parent: { realization: message.parentRealization, fingerprint: message.parentFingerprint }
}
```

Add the parent identity fields to the comparison host message emitted in Task
3. Clear both identities and detail state when commits or comparison mode
change.

- [ ] **Step 2: Request exact community comparisons**

When aggregate comparison activation occurs, derive side presence and member
counts from the selected aggregate node's evidence snapshots. Send
`compareCommunity`, retain its request ID plus commit/parent/community tuple,
and render loading state without unmounting the aggregate graph.

- [ ] **Step 3: Accept only current responses**

For `communityComparison`, require an exact match on request ID, selected
commit, comparison parent, and community. Parse optional graph sides. Synthesize
an empty graph from the present side when one side is intentionally absent,
then call `compareGraphs(parent, current)`.

Set bounded metadata when either loaded detail node count is less than the
corresponding aggregate `memberCount`. For a correlated error, keep the
aggregate graph and expose a retryable error. Back clears only detail/request
state.

- [ ] **Step 4: Wire revision-aware source opening**

Pass `{ before: comparison.parent, after: selected.commit }` to `CompassGraph`.
Forward a supplied revision to `HistoryHost.openSource`; otherwise retain the
selected commit fallback for ordinary historical graphs.

- [ ] **Step 5: Add regression coverage after implementation**

Extend `HistoryWorkspace.test.tsx` to verify `Inspect changes`, loading status,
detailed summary/filter counts, bounded notice, Back restoration, retry
behavior, and before/after source commit forwarding.

- [ ] **Step 6: Verify Task 4**

Run:

```bash
npm test -w @compass/viewer -- HistoryWorkspace
npm run build -w editors/vscode
npm run typecheck -w editors/vscode
```

Expected: component tests, webview bundle, and TypeScript all succeed.

---

### Task 5: Browser coverage, documentation, and release verification

**Files:**
- Modify: `tests/viewer/fixtures/generate.ts`
- Modify: `tests/viewer/history.spec.ts`
- Modify: `editors/vscode/README.md`
- Modify: `docs/guides/vscode.md`
- Modify: `docs/superpowers/specs/2026-07-26-vscode-community-diff-detail-design.md`

**Interfaces:**
- Consumes: completed feature from Tasks 1–4.
- Produces: end-to-end evidence, user guidance, and current graph metadata.

- [ ] **Step 1: Add a representative browser fixture**

Generate an aggregate comparison with one changed community, then provide
parent/current detail containing a changed signature/source range, one added
symbol, one removed symbol, and added/removed relationships.

- [ ] **Step 2: Add the browser workflow regression**

In `history.spec.ts`, open Changed graph, select the community, activate
`Inspect changes`, assert the symbol list and visible delta, select the changed
symbol, assert Before/After values and relationship statuses, and return with
Back.

- [ ] **Step 3: Document the interaction**

Update the extension README and VS Code guide with the lazy drill-down,
field-level evidence, relationship evidence, historical source actions, and
the `compass.graphNodeLimit` bounded-results notice. Change the design spec
status to `Implemented`.

- [ ] **Step 4: Run focused and full verification**

Run:

```bash
npm test -w @compass/viewer
npm test -w editors/vscode
npm run typecheck:js
npm run build:viewer
npm run build:vscode
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
npm test -w @compass/viewer-tests -- history.spec.ts
```

Expected: all checks, including the history Playwright workflow, pass without
new warnings.

- [ ] **Step 5: Refresh the knowledge graph and inspect the final diff**

Run:

```bash
graphify update .
git status --short
git diff --check
git diff --stat HEAD~1
```

Expected: the knowledge graph is current, formatting checks pass, and only
feature-related files are present.
