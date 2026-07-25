# VS Code Inspector and Call-Graph Loading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate exact source navigation and community identity in the graph inspector, and replace the raw call-graph resolver sentence with Compass’s balanced, recoverable loading experience.

**Architecture:** `GraphInspector` keeps its existing host callbacks but renders navigable source metadata as one native button and neighbor community colors as dots. `GraphLoadingState` gains optional loading copy so the call-graph webview can reuse the established constellation, accessibility, and error-recovery surface without duplicating a visual system. The call-graph host adds retry and output actions around its existing cursor-position query.

**Tech Stack:** TypeScript, React 19, Lucide React, VS Code Webview API, Compass viewer CSS, Vitest, Playwright.

## Global Constraints

- Implement behavior before adding regression coverage; do not use a TDD/red-green-refactor sequence.
- Keep Compass runtime artifacts under `compass-out/`; do not introduce a `graphify-out/` runtime path.
- Use existing VS Code theme tokens and the existing Compass constellation loader.
- Do not change graph contracts, source-range calculation, CLI query limits, or serializers.
- Preserve keyboard access, high-contrast behavior, and reduced-motion behavior.
- Do not stage generated output directories or unrelated user files.

---

### Task 1: Integrated Source Card and Neighbor Dots

**Context:** The inspector duplicates source information across metadata and a separate action, while neighbor community color is presented as a heavy left border.

**Goal:** Make the Source metadata surface the exact navigation action and use compact colored dots for neighbor identity.

**Expected outcome:** A selected node with source data exposes one clear Source button containing path, exact line range, and an ExternalLink icon. Connected-node rows have community-color dots and no colored border.

**Files:**

- Modify: `packages/compass-viewer/src/graph/GraphInspector.tsx`
- Modify: `packages/compass-viewer/src/theme.css`
- Test after implementation: `tests/viewer/graph-parity.spec.ts`

**Interfaces:**

- Consumes: `navigableSource(node): SourceLocation | undefined`
- Consumes: `onOpenSource(source: SourceLocation): void`
- Consumes: `onFocus(nodeId: string): void`
- Produces: `.compass-source-card`, `.compass-source-path`, `.compass-source-range`, and `.compass-neighbor-dot` presentation hooks
- Preserves: the exact `SourceLocation` object sent to `onOpenSource`

- [ ] **Step 1: Add source display helpers**

Keep `lineRange(node)` for compact metadata and add an accessible source action label:

```ts
function sourceActionLabel(node: GraphNode, source: SourceLocation): string {
  const start = node.source?.startLine;
  const end = node.source?.endLine;
  if (start === undefined) return `Open source ${source.file}`;
  if (end !== undefined && end !== start) {
    return `Open source ${source.file} at lines ${start}–${end}`;
  }
  return `Open source ${source.file} at line ${start}`;
}
```

Import `ExternalLinkIcon` from `lucide-react`.

- [ ] **Step 2: Replace the static Source metadata and duplicate action**

Inside the wide Source metadata entry:

```tsx
<div className="compass-metadata-wide compass-source-metadata">
  <dt>Source</dt>
  <dd>
    {source ? (
      <button
        className="compass-source-card"
        type="button"
        aria-label={sourceActionLabel(selected, source)}
        title={sourceActionLabel(selected, source)}
        onClick={() => onOpenSource(source)}
      >
        <span className="compass-source-copy">
          <span className="compass-source-path">{source.file}</span>
          {range && (
            <span className="compass-source-range">
              {range.includes("–") ? `Lines ${range}` : `Line ${range}`}
            </span>
          )}
        </span>
        <ExternalLinkIcon aria-hidden="true" />
      </button>
    ) : (
      <span>{selected.source?.file ?? "Not recorded"}</span>
    )}
  </dd>
</div>
```

Remove only the source-specific `.compass-inspector-action` button. Preserve the Open community action.

- [ ] **Step 3: Replace neighbor borders with dots**

Render each neighbor as:

```tsx
<button
  key={neighbor.id}
  type="button"
  className="compass-neighbor-link"
  title={neighbor.label}
  onClick={() => onFocus(neighbor.id)}
>
  <span
    className="compass-neighbor-dot"
    aria-hidden="true"
    style={{ background: neighbor.color?.background
      ?? model.communities.find((item) => item.id === neighbor.community)?.color
      ?? "var(--border)" }}
  />
  <span className="compass-neighbor-label">{neighbor.label}</span>
</button>
```

- [ ] **Step 4: Style the integrated controls**

In `theme.css`:

- remove Source-specific dependence on `.compass-inspector-action`;
- make `.compass-source-card` a full-width, borderless inner button with path/range columns and a trailing 15-pixel icon;
- add hover and focus border/background treatment to `.compass-source-metadata`;
- make `.compass-neighbor-link` a flex row with no left border;
- add an 8-pixel circular `.compass-neighbor-dot`;
- truncate `.compass-neighbor-label`;
- include `.compass-source-card` in focus-visible and high-contrast selectors.

- [ ] **Step 5: Build the viewer and extension**

Run:

```bash
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:viewer
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:vscode
```

Expected: both builds exit 0.

- [ ] **Step 6: Add post-implementation browser coverage**

Update `graph-parity.spec.ts` to assert:

```ts
const sourceCard = page.getByRole("button", {
  name: "Open source src/lib.rs at lines 5–7"
});
await expect(sourceCard).toBeVisible();
await expect(sourceCard.locator(".compass-source-path")).toHaveText("src/lib.rs");
await expect(sourceCard.locator(".compass-source-range")).toHaveText("Lines 5–7");
await expect(page.locator(".compass-neighbor-link")).not.toHaveCSS(
  "border-left-width",
  "3px"
);
await expect(page.locator(".compass-neighbor-dot")).toHaveCount(2);
```

Click the Source card in the web-export fixture and assert the existing
`compass:open-source` event receives:

```ts
{ file: "src/lib.rs", startLine: 5, endLine: 7 }
```

Keep the file-only README assertion, verify it still displays `README.md`, and
verify it has no Source button.

---

### Task 2: Balanced Call-Graph Resolver

**Context:** The call-graph panel initially renders only “Resolving the function under your cursor…” at the top-left. The main graph webview already has a centered, theme-aware, reduced-motion-safe constellation and recoverable error state.

**Goal:** Reuse the established loader with call-graph-specific progress copy and working Retry/Show output actions.

**Expected outcome:** Call-graph resolution opens with a centered Compass mark and calm graph constellation, explains the three useful phases, then transitions to the call graph. Errors retain the same balanced shell and expose recovery.

**Files:**

- Modify: `editors/vscode/src/webviews/GraphLoadingState.tsx`
- Modify: `editors/vscode/src/webviews/GraphLoadingState.test.tsx`
- Modify: `editors/vscode/src/webviews/callGraph.tsx`
- Modify: `editors/vscode/src/views/callGraphPanel.ts`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `tests/viewer/fixtures/generate.ts`
- Test after implementation: `tests/viewer/callflow.spec.ts`

**Interfaces:**

- Produces:

```ts
export type GraphLoadingCopy = {
  eyebrow: string;
  title: string;
  steps: readonly string[];
};
```

- Extends:

```ts
GraphLoadingState({
  state,
  loadingCopy?,
  onRetry,
  onShowOutput
})
```

- Webview-to-host messages: `{ type: "retry" }` and `{ type: "showOutput" }`
- Host dependency: `vscode.OutputChannel`

- [ ] **Step 1: Make loading copy contextual**

Add `GraphLoadingCopy` and a `DEFAULT_LOADING_COPY`:

```ts
const DEFAULT_LOADING_COPY: GraphLoadingCopy = {
  eyebrow: "Compass graph",
  title: "Mapping your codebase",
  steps: ["Reading graph", "Arranging relationships", "Preparing inspector"]
};
```

Use `loadingCopy ?? DEFAULT_LOADING_COPY` only in the loading branch. Preserve the current error copy and actions.

- [ ] **Step 2: Render the call-graph loading and error states**

In `webviews/callGraph.tsx`, import `GraphLoadingState` and define:

```ts
const CALL_GRAPH_LOADING_COPY = {
  eyebrow: "Compass call graph",
  title: "Resolving the function under your cursor",
  steps: ["Locating symbol", "Tracing callers", "Tracing callees"]
} as const;
```

Add:

```ts
function renderLoading(): void {
  root.render(
    <GraphLoadingState
      state={{ kind: "loading" }}
      loadingCopy={CALL_GRAPH_LOADING_COPY}
      onRetry={() => {
        graph = undefined;
        renderLoading();
        vscode.postMessage({ type: "retry" });
      }}
      onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
    />
  );
}
```

Render `GraphLoadingState` with `state={{ kind: "error", message }}` for host errors. Call `renderLoading()` before posting `ready`.

- [ ] **Step 3: Add host recovery actions**

Change:

```ts
CallGraphPanel.open(context, session, editor, output)
```

Handle `ready` and `retry` by rerunning the original cursor query. Handle
`showOutput` with `output.show(true)`. Continue using the panel
`AbortController` on disposal.

Update the call site in `extension.ts` to pass the existing Compass output
channel.

- [ ] **Step 4: Add post-implementation unit coverage**

Extend `GraphLoadingState.test.tsx` with contextual copy:

```tsx
<GraphLoadingState
  state={{ kind: "loading" }}
  loadingCopy={{
    eyebrow: "Compass call graph",
    title: "Resolving the function under your cursor",
    steps: ["Locating symbol", "Tracing callers", "Tracing callees"]
  }}
  onRetry={vi.fn()}
  onShowOutput={vi.fn()}
/>
```

Assert the contextual title and all three phases render while the default test
continues to assert “Mapping your codebase”.

- [ ] **Step 5: Add post-implementation browser coverage**

Replace the generic call-graph fixture harness with a call-graph-specific
harness that:

- delays successful hydration by 250 milliseconds;
- posts an error when the URL contains `?error`;
- records Retry and Show output messages.

Add Playwright assertions that:

```ts
await page.goto("/calls.html");
await expect(page.getByRole("status")).toContainText(
  "Resolving the function under your cursor"
);
await expect(page.getByText("Locating symbol")).toBeVisible();
await expect(page.getByText("Tracing callers")).toBeVisible();
await expect(page.getByText("Tracing callees")).toBeVisible();
await expect(page.getByText("Calls from run")).toBeVisible();
```

For `/calls.html?error`, assert the error surface exposes Retry and Show Compass
output and that both actions post their expected host messages.

- [ ] **Step 6: Run focused verification**

Run:

```bash
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run test -w @compass/viewer
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm test -w @compass/viewer-tests -- graph-parity.spec.ts callflow.spec.ts accessibility.spec.ts
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run typecheck:js
```

Expected: all focused tests and typechecks pass.

---

### Task 3: Final Verification and Publication

**Context:** Both changes ship inside the same VSIX and affect shared viewer assets.

**Goal:** Verify the complete extension, refresh repository graph metadata, commit only intended files, and update the existing PR.

**Expected outcome:** The branch contains reviewed implementation and post-implementation coverage, a fresh smoke-tested VSIX is available locally, and PR #38 includes the new commits.

**Files:**

- Review all modified files.
- Do not stage `.superpowers/`, `code-review-graph/`, `codegraph/`, `compass-out/`, or `graphify-out/`.

**Interfaces:**

- Existing branch: `agent/vscode-trust-correctness`
- Existing draft PR: `crabbuild/compass#38`

- [ ] **Step 1: Run the final matrix**

Run:

```bash
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run typecheck:js
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run test:js
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:viewer
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run build:vscode
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run test:integration -w editors/vscode
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run package -w editors/vscode
PATH=/tmp/compass-codex-node-runtime-20260725:$PATH npm run smoke:vsix -w editors/vscode
```

- [ ] **Step 2: Refresh the repository graph**

From `/Users/haipingfu/graphify`, run:

```bash
graphify update .
```

Do not stage generated graph output.

- [ ] **Step 3: Audit and commit explicit files**

Run:

```bash
git diff --check
git status --short
git diff --stat
```

Stage only the two design/plan documents, implementation files, and regression
coverage. Commit with:

```bash
git commit -m "feat(vscode): polish graph navigation states"
```

- [ ] **Step 4: Push and verify PR**

Push `agent/vscode-trust-correctness`, then confirm PR #38 still targets `main`
and contains the new commit.
