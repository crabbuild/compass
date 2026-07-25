# VS Code Trust and Correctness Implementation Plan

> **For Codex:** Use the executing-plans workflow to implement this plan task by task. This plan intentionally uses implementation-first sequencing; regression coverage is added after the behavior exists, per the project request.

**Goal:** Make historical graph browsing trustworthy under asynchronous updates, make revision builds recover cleanly from cancellation and failure, and disclose every UI truncation that can hide graph content.

**Architecture:** The VS Code history webview becomes the authoritative adapter for selected-commit presentation state. Every graph-derived message is commit-scoped and ignored when it does not match the current selection. The extension host reports an explicit build lifecycle keyed by commit and retains graph identities per revision so late loads cannot corrupt community navigation. Viewer components receive controlled state and render recovery/disclosure UI using VS Code theme tokens.

**Tech Stack:** TypeScript, React, VS Code Webview API, existing Compass viewer components, Playwright browser fixtures, VS Code integration tests.

**Constraints:**

- Do not use a TDD/red-green-refactor sequence.
- Implement each behavior before adding its regression coverage.
- Do not change Compass CLI limits or serializers.
- Runtime graph artifacts remain under `compass-out/`; do not introduce a `graphify-out/` runtime path.
- Preserve unrelated and untracked workspace files.

---

## Task 1: Make historical presentation state commit-scoped

**Context:** The history viewer currently owns a local selected commit while the VS Code adapter owns graph, comparison, counts, and community data. Because these states are independent, a late response for an old revision can overwrite the screen after the user selects a new revision.

**Goal:** Give the adapter one selected-commit authority and make every visible graph-derived value belong to that commit.

**Expected outcome:** Selecting a commit immediately clears the prior graph and derived information. Late graph, comparison, count, and community responses for another commit are ignored. The visible graph always labels its revision.

**Files:**

- Modify: `packages/compass-viewer/src/history/HistoryWorkspace.tsx`
- Modify: `packages/compass-viewer/src/history/CommitDetails.tsx`
- Modify: `packages/compass-viewer/src/index.ts`
- Modify: `editors/vscode/src/webviews/history.tsx`
- Create: `editors/vscode/src/history/panelMessages.ts`

### Step 1: Define the webview protocol

Create discriminated message types for:

- Timeline replacement.
- Revision graph loaded for a specific commit.
- Community graph loaded or failed for a specific commit.
- Comparison loaded for a specific commit.
- Change counts loaded for a specific commit.
- Build requesting/running/succeeded/failed/cancelled for a specific commit.
- Operation errors carrying an operation name and optional commit.

Define the webview-to-host requests in the same module so the two sides share commit identity and operation names.

### Step 2: Convert `HistoryWorkspace` to controlled revision state

Replace its internal selected-commit reducer ownership with props:

```ts
selectedCommit: string;
graphCommit?: string;
buildState?: HistoryBuildState;
operationError?: HistoryOperationError;
onSelectCommit(commit: string): void;
```

Keep local query/filter state in the viewer. Derive the selected entry from `selectedCommit`, and render a graph only when `graphCommit === selectedCommit`.

Add a compact context strip above a visible graph:

```text
Viewing graph for <short commit>
```

### Step 3: Make selection synchronous in the adapter

In `webviews/history.tsx`, add a single `selectCommit(commit)` transition that:

1. Sets `selectedCommit`.
2. Clears graph and graph identity.
3. Clears semantic comparison data.
4. Clears change counts and community drilldown data.
5. Clears selection-scoped operation errors.
6. Renders the empty/loading revision state.

When a refreshed timeline arrives:

- Retain the selected commit if it still exists.
- Otherwise select `selectedHead` when it exists.
- Otherwise select the first timeline entry.

### Step 4: Reject stale responses

For graph, comparison, counts, community, and community-error messages:

```ts
if (message.commit !== selectedCommit) {
  return;
}
```

Do not route operational failures into `semanticDiff`. Store and render them as operation errors.

### Step 5: Disable unavailable comparisons

Pass the set of materialized timeline commits to `CommitDetails`. A parent comparison is enabled only when:

- The selected revision has a presentation.
- The parent revision has a presentation.

Render a concise explanation for disabled comparison controls, such as “Build this revision first” or “Parent graph is not available.”

### Step 6: Build the affected packages

Run:

```bash
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:viewer
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:vscode
```

Expected: both packages compile without TypeScript or bundling errors.

---

## Task 2: Add explicit build recovery and race-safe host identity

**Context:** The current build command has a boolean “building” state and emits a generic error on failure. Profile/source cancellation can leave the UI ambiguous. The host also retains one active historical graph, so a late revision load can make a different visible revision use the wrong community identity.

**Goal:** Make every build terminal path explicit and keep historical graph identity keyed by commit.

**Expected outcome:** A build always leaves requesting/running state through success, failure, or cancellation. Failure is actionable, cancellation is neutral, and late loads cannot corrupt community navigation.

**Files:**

- Modify: `editors/vscode/src/views/historyPanel.ts`
- Use: `editors/vscode/src/history/panelMessages.ts`
- Modify: `packages/compass-viewer/src/history/CommitDetails.tsx`
- Modify: `packages/compass-viewer/src/history/HistoryWorkspace.tsx`
- Modify: `editors/vscode/src/webviews/history.tsx`

### Step 1: Model build state by commit in the webview

Use a map keyed by commit:

```ts
type HistoryBuildState =
  | { status: "requesting" }
  | { status: "running" }
  | { status: "failed"; message: string };
```

Set `requesting` synchronously before posting `buildRevision`. Apply host lifecycle events only to their matching commit. Remove the state after success or cancellation.

### Step 2: Report every host lifecycle transition

In the extension host:

- Post `buildRunning` after profile/source input is complete and before the CLI starts.
- Post `buildSucceeded` only after the command succeeds and the refreshed timeline is available.
- Post `buildFailed` with a concise error for writer conflicts, CLI errors, or timeline refresh failures.
- Post `buildCancelled` when profile selection, source input, or progress cancellation is dismissed.

Track progress cancellation separately from process exit so a killed command is not shown as a failure.

### Step 3: Render recovery near revision actions

In `CommitDetails`:

- Disable the build button while requesting/running.
- Show “Choosing profile…” for requesting.
- Show “Building…” for running.
- Show “Retry build” after failure.
- Render failure copy near the actions using `role="alert"`.
- Render cancellation as a neutral return to idle, without an error banner.

Operation errors for load, compare, counts, and community actions appear in the same revision context and never as semantic findings.

### Step 4: Replace single active historical graph identity

Replace the host’s single `activeGraph` value with a small commit-keyed identity cache containing:

```ts
{
  commit: string;
  realization: string;
  fingerprint: string;
}
```

Use the request commit to resolve the identity for community loads. Bound the cache to the existing revision cache scale so browsing many revisions does not retain every full graph.

### Step 5: Typecheck and build

Run:

```bash
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run typecheck:js
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:vscode
```

Expected: the shared protocol exhaustively covers the handled messages and the extension compiles.

---

## Task 3: Disclose architecture and call-graph truncation

**Context:** Architecture overview links are silently limited to 24 and call continuations to 20. Call graph contracts already expose a `truncated` flag, but the UI does not disclose it.

**Goal:** Ensure users can distinguish “complete” from “currently summarized” graph views and reveal all data already loaded in memory.

**Expected outcome:** Every client-side limit displays visible counts and a Show all action. A server-truncated call graph displays an explicit partial-results alert.

**Files:**

- Modify: `packages/compass-viewer/src/architecture/ArchitectureFlow.tsx`
- Modify: `packages/compass-viewer/src/calls/CallGraph.tsx`

### Step 1: Disclose architecture flow limits

Keep the initial 24-card rendering for layout performance. When more links exist, add:

```text
Showing 24 of N flows
[Show all N flows]
```

The action reveals all links already present in the overview response. Use existing Button and theme-token styling.

### Step 2: Disclose call graph size and partial results

Always show node and edge counts derived from the loaded graph.

When `graph.truncated` is true, render an alert:

```text
Partial call graph
Compass reached the configured graph limit. Counts and paths may be incomplete.
```

### Step 3: Disclose continuation limits

Keep the initial 20 continuation rows. When more are loaded, add:

```text
Showing 20 of N continuations
[Show all N continuations]
```

Do not change CLI query limits or fetch additional data in this PR.

### Step 4: Build the viewer

Run:

```bash
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:viewer
```

Expected: viewer components compile and retain existing design-system styling.

---

## Task 4: Add post-implementation regression coverage

**Context:** The trust fixes depend on event ordering and visible disclosure. These behaviors need browser-level protection after implementation.

**Goal:** Cover stale-event rejection, build recovery, comparison availability, and truncation disclosure without changing the implementation sequence to TDD.

**Expected outcome:** Regression tests fail if historical data leaks across selection, builds become stuck, unavailable comparisons become actionable, or summary limits become silent again.

**Files:**

- Modify: `tests/viewer/fixtures/generate.ts`
- Modify: `tests/viewer/history.spec.ts`
- Modify: `tests/viewer/callflow.spec.ts`
- Modify if needed: `editors/vscode/src/test/suite/extension.integration.ts`

### Step 1: Expand the history harness

Generate a timeline with:

- Two materialized revisions with visibly distinct graph content.
- One unmaterialized revision for build recovery.
- Available and unavailable parent relationships.

Support delayed revision responses so an older response can arrive after a newer selection. Support build cancellation, failure, and success lifecycle messages.

### Step 2: Cover stale historical events

Add a browser test that:

1. Requests revision A with a delayed response.
2. Selects and loads revision B.
3. Allows revision A to arrive.
4. Confirms only revision B’s graph and context strip are visible.

Also verify stale comparison/count/community messages cannot alter revision B.

### Step 3: Cover build recovery

Add scenarios for:

- Cancelled build returns the action to idle.
- Failed build shows the failure and Retry build.
- Successful build refreshes the timeline and clears the busy state.

### Step 4: Cover comparison availability

Confirm comparison controls are disabled with explanatory copy when either side lacks a presentation and become enabled after the revision is available.

### Step 5: Cover truncation disclosure

Generate:

- An architecture overview with 25 flows.
- A call graph with 21 continuations and `truncated: true`.

Assert the initial counts, partial-results alert, and Show all transitions.

### Step 6: Run focused coverage

Run:

```bash
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm test -w @compass/viewer-tests -- history.spec.ts callflow.spec.ts
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run test:integration -w editors/vscode
```

Expected: focused browser and extension integration coverage passes.

---

## Task 5: Verify, review, update the knowledge graph, and publish

**Context:** The change crosses viewer state, extension-host process lifecycle, and browser behavior. It should be verified at package and extension boundaries before publication.

**Goal:** Produce fresh evidence that the implementation is correct, review the final diff, keep the repository knowledge graph current, and publish an auditable draft PR.

**Expected outcome:** All relevant checks pass, only intended files are committed, the feature branch is pushed, and a draft PR summarizes trust behavior and verification.

**Files:**

- Review all modified files.
- Update existing project graph metadata via the repository-required command, without staging runtime/output directories.

### Step 1: Run the full relevant verification matrix

Run:

```bash
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run typecheck:js
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run test:js
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:viewer
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:vscode
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm test -w @compass/viewer-tests
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run test:integration -w editors/vscode
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run package -w editors/vscode
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run smoke:vsix -w editors/vscode
```

If a check fails, diagnose the failure, fix only in-scope causes, and rerun the failed check plus any affected upstream checks.

### Step 2: Review the final diff

Inspect:

```bash
git status --short
git diff --check
git diff --stat origin/main...HEAD
git diff origin/main...HEAD
```

Confirm:

- No runtime paths were changed from `compass-out/`.
- No unrelated untracked directories are staged.
- Errors are separate from semantic findings.
- Every async response is commit-scoped.
- Every build path reaches a terminal event.
- Every in-memory list limit is disclosed.

### Step 3: Request focused code review

Review the final branch specifically for:

- Async ordering and stale-response races.
- Cancellation/failure terminal paths.
- Accessibility of status, alerts, and disabled explanations.
- Retained large graph data or avoidable rerenders.

Address actionable issues and rerun affected verification.

### Step 4: Refresh repository knowledge metadata

From the repository root, run:

```bash
graphify update .
```

Do not stage `graphify-out/`, `compass-out/`, or other generated/untracked output directories.

### Step 5: Commit and publish a draft PR

Stage only the explicit implementation, coverage, and documentation files. Create scoped commits, push `agent/vscode-trust-correctness`, and open a draft PR against `main`.

PR body sections:

- Context: why stale history and ambiguous builds reduce trust.
- Changes: synchronization, build recovery, truncation disclosure.
- Validation: exact commands and results.
- Scope boundaries: no CLI limit, serializer, or runtime output-path changes.

