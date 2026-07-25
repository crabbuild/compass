# VS Code-native Compass viewer UX implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every Compass webview readable, responsive, and fully operable in VS Code light, dark, and high-contrast themes.

**Architecture:** Keep CLI execution and request cancellation in the VS Code extension host. Put reusable theme aliases, collection-state utilities, and UI components in `@compass/viewer`; keep entry-point loading and host-message translation in the VS Code webviews. Preserve existing schemas and add only presentation state or additive messages.

**Tech Stack:** TypeScript 5.9, React 19, VS Code Extension API 1.95, Zod 4, Vitest 3, Testing Library, Tailwind CSS 4, Vite 7, esbuild 0.25, and Playwright.

## Global constraints

- Every ordinary surface, control, border, state, and focus ring follows VS Code semantic theme tokens
- Light, dark, and high-contrast themes remain readable without a separate Compass theme
- Compass branding appears only in the product mark and graph data colors
- Preserve the current graph, call-flow, query, and history schemas
- Preserve unfinished Graph, Call Graph, loading, inspector, and viewer-theme work already committed on this branch
- Implement each bounded behavior before adding its targeted regression and interaction coverage
- Default Architecture page sizes are 24 symbols and 25 calls
- Selecting an available historical commit loads its graph automatically
- Selecting an unavailable historical commit never starts a build without an explicit **Build graph** action
- Narrow editor columns preserve every core action
- Reduced-motion and VS Code high-contrast modes receive equivalent behavior
- Run `graphify update .` after code changes

## File structure

New files have one responsibility:

- `packages/compass-viewer/src/lib/collectionView.ts`: generic filtering-independent pagination and page clamping
- `packages/compass-viewer/src/lib/collectionView.test.ts`: collection utility behavior
- `packages/compass-viewer/src/components/workbench/CollectionToolbar.tsx`: shared search and result count
- `packages/compass-viewer/src/components/workbench/Pagination.tsx`: shared page navigation
- `packages/compass-viewer/src/components/workbench/WorkspaceState.tsx`: shared empty, running, error, and unavailable presentation
- `packages/compass-viewer/src/architecture/state.ts`: Architecture global search, section filtering, call sorting, and result grouping
- `packages/compass-viewer/src/architecture/state.test.ts`: Architecture state behavior
- `packages/compass-viewer/src/query/state.ts`: structured query result normalization
- `packages/compass-viewer/src/query/state.test.ts`: query normalization behavior
- `tests/viewer/theme.spec.ts`: semantic-token, high-contrast, reduced-motion, and responsive integration checks

Existing component files remain responsible for composing their workspaces. Do not move CLI or webview messaging into `@compass/viewer`.

---

### Task 1: Semantic theme foundation and collection primitives

**Files:**

- Create: `packages/compass-viewer/src/lib/collectionView.ts`
- Create: `packages/compass-viewer/src/lib/collectionView.test.ts`
- Create: `packages/compass-viewer/src/components/workbench/CollectionToolbar.tsx`
- Create: `packages/compass-viewer/src/components/workbench/Pagination.tsx`
- Create: `packages/compass-viewer/src/components/workbench/WorkspaceState.tsx`
- Modify: `packages/compass-viewer/src/theme.css`
- Modify: `packages/compass-viewer/src/index.ts`

**Interfaces:**

- Consumes: VS Code CSS custom properties injected into webview documents
- Produces: `Page<T>`, `paginate`, `clampPage`, `CollectionToolbar`, `Pagination`, and `WorkspaceState`

- [ ] **Step 1: Implement the collection utilities**

Create:

```ts
export type Page<T> = {
  items: T[];
  page: number;
  pageCount: number;
  pageSize: number;
  total: number;
  start: number;
  end: number;
};

export function clampPage(page: number, pageCount: number): number {
  return Math.min(Math.max(1, Math.trunc(page) || 1), Math.max(1, pageCount));
}

export function paginate<T>(items: readonly T[], page: number, pageSize: number): Page<T> {
  const safePageSize = Math.max(1, Math.trunc(pageSize) || 1);
  const pageCount = Math.max(1, Math.ceil(items.length / safePageSize));
  const safePage = clampPage(page, pageCount);
  const offset = (safePage - 1) * safePageSize;
  const visible = items.slice(offset, offset + safePageSize);
  return {
    items: visible,
    page: safePage,
    pageCount,
    pageSize: safePageSize,
    total: items.length,
    start: visible.length === 0 ? 0 : offset + 1,
    end: offset + visible.length
  };
}
```

- [ ] **Step 2: Add collection utility coverage**

Add these cases to `collectionView.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { clampPage, paginate } from "./collectionView";

describe("collection view", () => {
  it("returns the requested page and visible range", () => {
    expect(paginate([1, 2, 3, 4, 5], 2, 2)).toEqual({
      items: [3, 4],
      page: 2,
      pageCount: 3,
      pageSize: 2,
      total: 5,
      start: 3,
      end: 4
    });
  });

  it("clamps empty and out-of-range pages", () => {
    expect(clampPage(0, 0)).toBe(1);
    expect(paginate(["a"], 9, 25).page).toBe(1);
  });
});
```

- [ ] **Step 3: Run the collection tests**

Run:

```bash
npm test -w @compass/viewer -- src/lib/collectionView.test.ts
```

Expected: PASS.

- [ ] **Step 4: Add shared workbench components**

Implement these exact public props:

```tsx
export function CollectionToolbar(props: {
  value: string;
  label: string;
  placeholder: string;
  resultCount: number;
  onChange(value: string): void;
}): JSX.Element;

export function Pagination(props: {
  page: number;
  pageCount: number;
  start: number;
  end: number;
  total: number;
  label: string;
  onPageChange(page: number): void;
}): JSX.Element;

export function WorkspaceState(props: {
  kind: "empty" | "running" | "error" | "unavailable";
  title: string;
  description: string;
  action?: { label: string; onClick(): void };
}): JSX.Element;
```

Use existing `Button` and `Input` components. `Pagination` disables previous on page 1 and next on the last page. `WorkspaceState` uses `role="status"` for `running`, `role="alert"` for `error`, and a neutral group for other states.

- [ ] **Step 5: Replace the incomplete theme aliases**

In `theme.css`, define aliases for every role used by shared and existing components:

```css
:root {
  --background: var(--vscode-editor-background, #1e1e1e);
  --foreground: var(--vscode-editor-foreground, #cccccc);
  --card: var(--vscode-editorWidget-background, var(--background));
  --card-foreground: var(--vscode-editorWidget-foreground, var(--foreground));
  --sidebar: var(--vscode-sideBar-background, var(--background));
  --sidebar-foreground: var(--vscode-sideBar-foreground, var(--foreground));
  --sidebar-accent: var(--vscode-list-activeSelectionBackground, #04395e);
  --sidebar-accent-foreground: var(--vscode-list-activeSelectionForeground, #ffffff);
  --sidebar-primary: var(--vscode-focusBorder, #007fd4);
  --muted: var(--vscode-list-inactiveSelectionBackground, transparent);
  --muted-foreground: var(--vscode-descriptionForeground, #9d9d9d);
  --border: var(--vscode-panel-border, #3c3c3c);
  --input: var(--vscode-input-background, var(--background));
  --ring: var(--vscode-focusBorder, #007fd4);
}
```

Remove ordinary chrome gradients, glass blur, and fixed light foregrounds. Keep graph community colors, graph canvas effects, and the Compass product mark. Add `.workbench-*` styles with 3 to 6 px radii, VS Code button tokens, 2 px focus outlines, and contrast-border overrides.

- [ ] **Step 6: Export the collection utilities and components**

Add exports to `packages/compass-viewer/src/index.ts`:

```ts
export * from "./lib/collectionView";
export * from "./components/workbench/CollectionToolbar";
export * from "./components/workbench/Pagination";
export * from "./components/workbench/WorkspaceState";
```

- [ ] **Step 7: Verify Task 1**

Run:

```bash
npm test -w @compass/viewer
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
```

Expected: all commands pass.

- [ ] **Step 8: Commit Task 1**

```bash
git add packages/compass-viewer/src/lib packages/compass-viewer/src/components/workbench packages/compass-viewer/src/theme.css packages/compass-viewer/src/index.ts
git commit -m "feat(viewer): add VS Code-native workbench primitives"
```

---

### Task 2: Balanced graph and Architecture loading

**Files:**

- Modify: `editors/vscode/src/webviews/GraphLoadingState.tsx`
- Modify: `editors/vscode/src/webviews/GraphLoadingState.test.tsx`
- Modify: `editors/vscode/src/webviews/architecture.tsx`
- Modify: `editors/vscode/src/views/architecturePanel.ts`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `packages/compass-viewer/src/theme.css`
- Modify: `tests/viewer/fixtures/generate.ts`
- Modify: `tests/viewer/callflow.spec.ts`

**Interfaces:**

- Consumes: existing `GraphLoadingState`, `GraphLoadingCopy`, Architecture `ready`, `hydrate`, `error`, and source-navigation messages
- Produces: optional `variant: "graph" | "architecture"` and Architecture `retry` and `showOutput` messages

- [ ] **Step 1: Implement the compact loader variants**

Add `variant?: "graph" | "architecture"` to `GraphLoadingState`. Replace the 240 by 180 constellation with a centered 48 px mark and native progress line. Render:

```tsx
{variant === "architecture" && loading && (
  <div className="architecture-load-skeleton" aria-hidden="true">
    <span className="architecture-load-rail" />
    <span className="architecture-load-flow" />
    <span className="architecture-load-content" />
  </div>
)}
```

Keep the current retry and output actions. Preserve the Call Graph purpose-specific copy. Update CSS so the loader copy and mark share one optical center and reduced-motion disables nonessential animation.

- [ ] **Step 2: Integrate loader and recovery into the Architecture webview**

In `architecture.tsx`, render the Architecture loader before posting `ready`, render `GraphLoadingState` on error, and send `retry` or `showOutput`.

In `architecturePanel.ts`:

- Treat `ready` and `retry` as hydration requests
- Increment a request generation before each export
- Ignore stale or aborted responses
- Append full errors to the Compass output channel
- Handle `showOutput` with `output.show(true)`

Change `openArchitecturePanel` to accept `output: vscode.OutputChannel`, and update the extension command registration to pass the existing output channel.

- [ ] **Step 3: Update the Architecture fixture**

Replace the generic Architecture harness with one that records messages, waits 800 ms when `delay=1`, emits `Architecture export failed` when `error=1`, and hydrates normally otherwise.

- [ ] **Step 4: Add loader component coverage**

Extend `GraphLoadingState.test.tsx`:

```tsx
it("renders a compact Architecture loader with a layout skeleton", () => {
  const markup = renderToStaticMarkup(
    <GraphLoadingState
      state={{ kind: "loading" }}
      variant="architecture"
      loadingCopy={{
        eyebrow: "Compass architecture",
        title: "Deriving architecture flow",
        steps: ["Reading graph", "Deriving subsystem flows", "Preparing symbol index"]
      }}
      onRetry={vi.fn()}
      onShowOutput={vi.fn()}
    />
  );
  expect(markup).toContain("compass-load-mark");
  expect(markup).toContain("architecture-load-skeleton");
  expect(markup).toContain("Deriving subsystem flows");
});
```

- [ ] **Step 5: Add Architecture loading integration coverage**

Extend `tests/viewer/callflow.spec.ts`:

```ts
test("architecture loading is informative and recoverable", async ({ page }) => {
  await page.goto("/architecture.html?delay=1");
  await expect(page.getByRole("heading", { name: "Deriving architecture flow" })).toBeVisible();
  await expect(page.getByRole("status")).toContainText("Preparing symbol index");
  await expect(page.locator(".architecture-load-skeleton")).toBeVisible();

  await page.goto("/architecture.html?error=1");
  await expect(page.getByRole("alert")).toContainText("Architecture export failed");
  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { architectureHostMessages: Array<{ type: string }> }
  ).architectureHostMessages.map(({ type }) => type))).toContain("retry");
});
```

- [ ] **Step 6: Verify and commit Task 2**

Run:

```bash
npm test -w editors/vscode -- src/webviews/GraphLoadingState.test.tsx
npm run typecheck -w editors/vscode
npm run build -w editors/vscode
npx playwright test -c tests/viewer/playwright.config.ts tests/viewer/callflow.spec.ts
```

Expected: all commands pass.

Commit:

```bash
git add editors/vscode/src/webviews/GraphLoadingState.tsx editors/vscode/src/webviews/GraphLoadingState.test.tsx editors/vscode/src/webviews/architecture.tsx editors/vscode/src/views/architecturePanel.ts editors/vscode/src/extension.ts packages/compass-viewer/src/theme.css tests/viewer/fixtures/generate.ts tests/viewer/callflow.spec.ts
git commit -m "feat(vscode): add native Architecture loading states"
```

---

### Task 3: Global Architecture search, symbol cards, and paginated calls

**Files:**

- Create: `packages/compass-viewer/src/architecture/state.ts`
- Create: `packages/compass-viewer/src/architecture/state.test.ts`
- Modify: `packages/compass-viewer/src/architecture/ArchitectureFlow.tsx`
- Modify: `packages/compass-viewer/src/theme.css`
- Modify: `tests/viewer/fixtures/generate.ts`
- Modify: `tests/viewer/callflow.spec.ts`
- Modify: `tests/viewer/accessibility.spec.ts`

**Interfaces:**

- Consumes: `CallflowViewModel`, `paginate`, `CollectionToolbar`, and `Pagination`
- Produces: `searchArchitecture`, `filterSectionSymbols`, `filterSectionCalls`, `sortCalls`, and interactive global results grouped by section

- [ ] **Step 1: Implement Architecture state**

Use these public types:

```ts
export type ArchitectureResult = {
  id: string;
  kind: "section" | "symbol" | "call";
  sectionId: string;
  sectionName: string;
  label: string;
  detail: string;
  tab: "symbols" | "calls";
};

export type ArchitectureResultGroup = {
  sectionId: string;
  sectionName: string;
  results: ArchitectureResult[];
};

export type CallSort = {
  column: "caller" | "relation" | "callee" | "confidence";
  direction: "ascending" | "descending";
};
```

Normalize search with `trim().toLocaleLowerCase()`. Match section, symbol label, kind, source path, caller, callee, relation, and confidence. Deduplicate results by `kind`, section, and source identifier. Return sections in model order and results in label order.

- [ ] **Step 2: Rebuild `ArchitectureFlow` around explicit state**

Use state for:

```ts
const [activeTab, setActiveTab] = useState<"symbols" | "calls">("symbols");
const [globalQuery, setGlobalQuery] = useState("");
const [symbolQuery, setSymbolQuery] = useState("");
const [callQuery, setCallQuery] = useState("");
const [symbolPage, setSymbolPage] = useState(1);
const [callPage, setCallPage] = useState(1);
const [callSort, setCallSort] = useState<CallSort>({
  column: "caller",
  direction: "ascending"
});
```

Render a global search combobox above the main content. Group results by subsystem. Selecting a result sets `sectionId`, `activeTab`, the relevant local filter, and page 1.

Render 24 paginated `.architecture-symbol-card` articles. Each card shows name, kind, subsystem, and a source button.

Render 25 paginated calls with sticky semantic headers. Header buttons update `aria-sort` and `callSort`. Evidence badges retain text labels. Keep the existing section rail and system-flow overview.

- [ ] **Step 3: Add Architecture-specific native CSS**

Replace Tailwind-dependent card colors with explicit `.architecture-*` classes backed by semantic aliases. Add:

- Sticky table headers
- Truncated paths with full `title` values
- Selected-section list styling
- Global search result popup using menu tokens
- Narrow-width stacked rail
- High-contrast active borders
- Horizontal table scrolling below the table's minimum readable width

- [ ] **Step 4: Add Architecture state coverage**

Create tests that use two sections with duplicate symbol labels:

```ts
it("groups global symbol and call matches by subsystem", () => {
  const groups = searchArchitecture(model, "database");
  expect(groups.map((group) => group.sectionName)).toEqual(["API", "Storage"]);
  expect(groups.flatMap((group) => group.results).map((result) => result.kind))
    .toEqual(expect.arrayContaining(["symbol", "call"]));
});

it("filters calls by resolved caller and callee labels", () => {
  const names = new Map([["a", "authenticate"], ["b", "database"]]);
  expect(filterSectionCalls(section, names, "database")).toHaveLength(1);
});

it("sorts call labels without mutating the source", () => {
  const source = [...section.edges];
  const sorted = sortCalls(section.edges, names, "callee", "ascending");
  expect(section.edges).toEqual(source);
  expect(sorted.map((edge) => names.get(edge.target))).toEqual(["cache", "database"]);
});
```

- [ ] **Step 5: Add Architecture interaction coverage**

Expand the Architecture fixture to include at least 31 symbols and 53 calls across `API` and `Storage`. Add Playwright assertions:

```ts
await page.getByRole("searchbox", { name: "Search architecture" }).fill("database");
await expect(page.getByRole("group", { name: "Storage search results" })).toBeVisible();
await page.getByRole("option", { name: /database.*symbol/i }).click();
await expect(page.getByRole("heading", { name: "Storage" })).toBeVisible();

await expect(page.locator(".architecture-symbol-card")).toHaveCount(24);
await page.getByRole("button", { name: "Next symbols page" }).click();
await expect(page.getByText("25–31 of 31 symbols")).toBeVisible();

await page.getByRole("tab", { name: "Calls" }).click();
await page.getByRole("searchbox", { name: "Filter Storage calls" }).fill("authenticate");
await expect(page.getByRole("row", { name: /authenticate.*database/i })).toBeVisible();
await expect(page.getByText(/1–25 of 53 calls/)).toBeVisible();
```

- [ ] **Step 6: Run Architecture tests and accessibility checks**

Run:

```bash
npm test -w @compass/viewer -- src/architecture/state.test.ts
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
npx playwright test -c tests/viewer/playwright.config.ts tests/viewer/callflow.spec.ts tests/viewer/accessibility.spec.ts
```

Expected: all commands pass.

- [ ] **Step 7: Commit Task 3**

```bash
git add packages/compass-viewer/src/architecture packages/compass-viewer/src/theme.css tests/viewer/fixtures/generate.ts tests/viewer/callflow.spec.ts tests/viewer/accessibility.spec.ts
git commit -m "feat(viewer): make Architecture Flow searchable"
```

---

### Task 4: VS Code-native Ask Codebase workspace

**Files:**

- Create: `packages/compass-viewer/src/query/state.ts`
- Create: `packages/compass-viewer/src/query/state.test.ts`
- Modify: `packages/compass-viewer/src/query/QueryWorkspace.tsx`
- Modify: `packages/compass-viewer/src/theme.css`
- Modify: `tests/viewer/fixtures/generate.ts`
- Create: `tests/viewer/query.spec.ts`
- Modify: `tests/viewer/accessibility.spec.ts`

**Interfaces:**

- Consumes: `QueryResult`, `WorkspaceState`, existing Query host `execute` and `cancel`
- Produces: `normalizeStructuredResult(value): StructuredResult | undefined`

- [ ] **Step 1: Implement result normalization**

Define:

```ts
export type StructuredResult = {
  columns: string[];
  rows: string[][];
};

export function normalizeStructuredResult(value: unknown): StructuredResult | undefined;
```

Accept a `{ rows: Record<string, unknown>[] }` payload only when every row is a plain object. Build columns in first-seen order. Render `null` as `null`, primitives with `String`, and nested values with `JSON.stringify`.

- [ ] **Step 2: Recompose `QueryWorkspace`**

Implement:

- A compact header with revision context
- A semantic segmented control for Natural Language and CompassQL
- Editor-style textarea and shortcut hint
- **Run query** and **Cancel query** accessible labels
- Natural-language example prompts
- Local query-history buttons that populate without executing
- `WorkspaceState` for running and error presentation
- A semantic structured table from `normalizeStructuredResult`
- Raw formatted JSON fallback

Keep request defaults at `timeoutMs: 5000` and `maxRows: 1000`. Focus the editor after an error action.

- [ ] **Step 3: Add Query fixture scenarios and native CSS**

Replace the generic query harness with one that records requests and emits:

- `state` with `running: true` for `delay=1`
- `result` with text for `result=text`
- `result` with object rows for `result=rows`
- `error` for `error=1`
- idle state on cancel

Style `.query-*` classes with input, editor, panel, button, progress, and table tokens. Add a 760 px breakpoint that stacks the Run action below the editor.

- [ ] **Step 4: Add structured-result coverage**

Create:

```ts
it("normalizes consistent object rows into columns", () => {
  expect(normalizeStructuredResult({
    rows: [{ symbol: "run", calls: 3 }, { symbol: "save", calls: 2 }]
  })).toEqual({
    columns: ["symbol", "calls"],
    rows: [["run", "3"], ["save", "2"]]
  });
});

it("returns undefined for irregular or non-row payloads", () => {
  expect(normalizeStructuredResult({ rows: [["a"], { name: "b" }] })).toBeUndefined();
  expect(normalizeStructuredResult({ value: 1 })).toBeUndefined();
});
```

- [ ] **Step 5: Add Query interaction tests**

Create `tests/viewer/query.spec.ts` with fixture scenarios for natural text, structured rows, error, and delayed cancellation:

```ts
test("query supports keyboard execution and cancellation", async ({ page }) => {
  await page.goto("/query.html?delay=1");
  const editor = page.getByRole("textbox", { name: "Natural-language query" });
  await editor.fill("How does authentication reach storage?");
  await editor.press(process.platform === "darwin" ? "Meta+Enter" : "Control+Enter");
  await expect(page.getByRole("button", { name: "Cancel query" })).toBeVisible();
  await page.getByRole("button", { name: "Cancel query" }).click();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { queryHostMessages: Array<{ type: string }> }
  ).queryHostMessages.at(-1)?.type)).toBe("cancel");
});

test("query renders structured columns and recoverable errors", async ({ page }) => {
  await page.goto("/query.html?result=rows");
  await page.getByRole("textbox", { name: "Natural-language query" }).fill("List symbols");
  await page.getByRole("button", { name: "Run query" }).click();
  await expect(page.getByRole("columnheader", { name: "symbol" })).toBeVisible();
  await expect(page.getByRole("cell", { name: "run" })).toBeVisible();
});
```

- [ ] **Step 6: Verify and commit Task 4**

Run:

```bash
npm test -w @compass/viewer -- src/query/state.test.ts
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
npx playwright test -c tests/viewer/playwright.config.ts tests/viewer/query.spec.ts tests/viewer/accessibility.spec.ts
```

Expected: all commands pass.

Commit:

```bash
git add packages/compass-viewer/src/query packages/compass-viewer/src/theme.css tests/viewer/fixtures/generate.ts tests/viewer/query.spec.ts tests/viewer/accessibility.spec.ts
git commit -m "feat(viewer): redesign Ask Codebase for VS Code"
```

---

### Task 5: Automatic and usable revision selection

**Files:**

- Modify: `editors/vscode/src/webviews/history.tsx`
- Modify: `packages/compass-viewer/src/history/HistoryWorkspace.tsx`
- Modify: `packages/compass-viewer/src/history/CommitRail.tsx`
- Modify: `packages/compass-viewer/src/history/CommitDetails.tsx`
- Modify: `packages/compass-viewer/src/theme.css`
- Modify: `tests/viewer/fixtures/generate.ts`
- Modify: `tests/viewer/history.spec.ts`
- Modify: `tests/viewer/accessibility.spec.ts`

**Interfaces:**

- Consumes: existing history timeline and revision host messages
- Produces: `RevisionLoadState = "idle" | "loading" | "ready"` and automatic `loadRevision` requests for available selections

- [ ] **Step 1: Add explicit revision load state to the webview**

In `history.tsx`, add:

```ts
type RevisionLoadState = "idle" | "loading" | "ready";
let revisionLoadState: RevisionLoadState = "idle";

function requestSelectedRevision(commit: string): void {
  const entry = timeline?.entries.find((candidate) => candidate.commit === commit);
  if (!entry?.presentationAvailable) {
    revisionLoadState = "idle";
    return;
  }
  revisionLoadState = "loading";
  operationErrors.delete(commit);
  postMessage({ type: "loadRevision", commit });
}
```

Call `requestSelectedRevision` after initial timeline selection, available commit selection, and successful-build timeline refresh. Set `ready` only after an accepted graph or comparison message. Set `idle` on accepted load errors.

Pass `revisionLoadState` into `HistoryWorkspace`. Keep commit identity checks before mutating any graph, comparison, count, or community state.

- [ ] **Step 2: Recompose Evolution loading and unavailable states**

In `HistoryWorkspace`, remove the **Open graph** action and the generic graph placeholder. Render:

- A loading `WorkspaceState` titled `Loading <subject>` while `revisionLoadState === "loading"`
- A ready graph when `graphCommit === selected.commit`
- An unavailable `WorkspaceState` titled `Graph not built for this revision` with **Build graph**
- An error state from the selected operation error with **Retry load** when the graph exists

Move build action ownership from `CommitDetails` into the graph-area state. Keep Query and Compare actions in `CommitDetails` when their prerequisites exist.

- [ ] **Step 3: Make the timeline informative and responsive**

Update `CommitRail` rows to show:

- Subject
- Short hash and author
- Formatted date through `Intl.DateTimeFormat`
- Text graph-state label beside the icon

Use the scroll container's actual `clientHeight` instead of the fixed `520` viewport when calculating visible rows. At widths below 760 px, render a compact native `<select aria-label="Select revision">` above details and keep the virtual rail hidden.

- [ ] **Step 4: Update fixtures for visible loading**

Delay every fixture graph response by 120 ms. Keep Revision A at 180 ms for stale-response coverage. When a successful build refreshes the timeline, expect the webview to request Revision C automatically.

- [ ] **Step 5: Add automatic-load interaction tests**

Update `history.spec.ts`:

```ts
test("available selection loads its graph automatically", async ({ page }) => {
  await page.goto("/history.html");
  await expect(page.getByText(/Viewing graph for aaaaaaaaa/)).toBeVisible();
  await page.getByRole("option", { name: /Revision B graph/i }).click();
  await expect(page.getByRole("status")).toContainText("Loading Revision B graph");
  await expect(page.getByText(/Viewing graph for bbbbbbbbb/)).toBeVisible();
  await expect.poll(() => page.evaluate(() => (
    window as typeof window & { historyHostMessages: Array<Record<string, unknown>> }
  ).historyHostMessages.filter(({ type }) => type === "loadRevision").length)).toBe(2);
});

test("unavailable selection presents build without starting it", async ({ page }) => {
  await page.goto("/history.html");
  await page.getByRole("option", { name: /Revision C needs build/i }).click();
  await expect(page.getByRole("heading", { name: "Graph not built for this revision" })).toBeVisible();
  expect(await page.evaluate(() => (
    window as typeof window & { historyHostMessages: Array<Record<string, unknown>> }
  ).historyHostMessages.some(({ type }) => type === "buildRevision"))).toBe(false);
});
```

- [ ] **Step 6: Run history behavior and accessibility tests**

Run:

```bash
npm run typecheck -w @compass/viewer
npm run typecheck -w editors/vscode
npx playwright test -c tests/viewer/playwright.config.ts tests/viewer/history.spec.ts tests/viewer/accessibility.spec.ts
```

Expected: all commands pass.

- [ ] **Step 7: Commit Task 5**

```bash
git add editors/vscode/src/webviews/history.tsx packages/compass-viewer/src/history packages/compass-viewer/src/theme.css tests/viewer/fixtures/generate.ts tests/viewer/history.spec.ts tests/viewer/accessibility.spec.ts
git commit -m "feat(vscode): make Evolution a revision browser"
```

---

### Task 6: Recoverable Evolution host bootstrap and build lifecycle

**Files:**

- Modify: `editors/vscode/src/views/historyPanel.ts`
- Modify: `editors/vscode/src/history/panelMessages.ts`
- Modify: `editors/vscode/src/webviews/history.tsx`
- Create: `editors/vscode/src/history/panelMessages.test.ts`
- Modify: `editors/vscode/src/test/suite/extension.integration.ts`
- Modify: `tests/viewer/fixtures/generate.ts`
- Modify: `tests/viewer/history.spec.ts`

**Interfaces:**

- Consumes: existing `loadTimeline`, `RevisionStore`, build progress, and output channel
- Produces: panel-level `{ type: "bootstrapError"; message: string }` and webview `{ type: "retryTimeline" }`

- [ ] **Step 1: Add the bootstrap message types**

Add `"Load history"` to `HistoryOperation`. Add `{ type: "retryTimeline" }` after `{ type: "ready" }` in `HistoryWebviewMessage`. Add `{ type: "bootstrapError"; message: string }` after the timeline variant in `HistoryHostMessage`.

Map `retryTimeline` to `Load history` in `historyOperationFor`.

- [ ] **Step 2: Make timeline loading happen after webview readiness**

Refactor `openHistoryPanel` so panel HTML and message handlers exist before timeline work. Use:

```ts
let timeline: HistoryTimeline | undefined;

async function sendTimeline(): Promise<void> {
  try {
    timeline = await loadTimeline(session);
    await postMessage({ type: "timeline", timeline, repositoryId: session.id });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    output.appendLine(`[history:error] ${message}`);
    await postMessage({ type: "bootstrapError", message });
  }
}
```

Call `sendTimeline()` for `ready` and `retryTimeline`. Guard all later handlers with a concrete timeline. Keep build refreshes posting the new timeline.

- [ ] **Step 3: Render bootstrap recovery**

In `history.tsx`, render `WorkspaceState` when `bootstrapError` arrives. Its action posts `retryTimeline` and restores a loading state. Clear the bootstrap error after a valid timeline.

Keep build cancellation explicit. A cancelled profile picker restores **Build graph** without an alert. A running cancellation clears the busy state and never marks the revision failed.

- [ ] **Step 4: Add recovery fixture behavior**

Add `history.html?bootstrap=error` behavior: the first `ready` emits `bootstrapError`; `retryTimeline` emits the normal timeline.

- [ ] **Step 5: Add message and recovery coverage**

Extend `panelMessages.test.ts`:

```ts
it("accepts retryTimeline without a commit", () => {
  expect(historyOperationFor({ type: "retryTimeline" })).toBe("Load history");
});

it("labels bootstrap failures as history loading", () => {
  const message: HistoryHostMessage = {
    type: "bootstrapError",
    message: "Git history is unavailable"
  };
  expect(message.type).toBe("bootstrapError");
});
```

Add this Playwright interaction:

```ts
await page.goto("/history.html?bootstrap=error");
await expect(page.getByRole("alert")).toContainText("Git history is unavailable");
await page.getByRole("button", { name: "Retry history" }).click();
await expect(page.getByRole("listbox", { name: "Git commit timeline" })).toBeVisible();
```

Extend the extension integration test to assert that timeline failure leaves the created panel alive and logs the full error.

- [ ] **Step 6: Verify and commit Task 6**

Run:

```bash
npm test -w editors/vscode
npm run typecheck -w editors/vscode
npm run build -w editors/vscode
npx playwright test -c tests/viewer/playwright.config.ts tests/viewer/history.spec.ts
```

Expected: all commands pass.

Commit:

```bash
git add editors/vscode/src/views/historyPanel.ts editors/vscode/src/history/panelMessages.ts editors/vscode/src/webviews/history.tsx editors/vscode/src/history/panelMessages.test.ts editors/vscode/src/test/suite/extension.integration.ts tests/viewer/fixtures/generate.ts tests/viewer/history.spec.ts
git commit -m "fix(vscode): make Evolution failures recoverable"
```

---

### Task 7: Cross-theme, responsive, and release verification

**Files:**

- Create: `tests/viewer/theme.spec.ts`
- Modify: `tests/viewer/fixtures/generate.ts`
- Modify: `tests/viewer/accessibility.spec.ts`
- Modify: `crates/compass-output/assets/viewer/viewer.css`
- Modify: `crates/compass-output/assets/viewer/graph.js`
- Modify: `crates/compass-output/assets/viewer/manifest.json`
- Modify: `editors/vscode/dist/webviews/viewer.css`

**Interfaces:**

- Consumes: all completed view components and fixture harnesses
- Produces: verified semantic-token behavior, embedded viewer assets, extension build output, and an updated graphify knowledge graph

- [ ] **Step 1: Complete cross-theme CSS and fixture setup**

Ensure fixture pages can inject VS Code variables before styles compute. Add body-class scenarios for `vscode-light`, `vscode-dark`, `vscode-high-contrast`, and `vscode-high-contrast-light`.

Audit `theme.css` with:

```bash
rg -n -- '--sidebar|#[0-9a-fA-F]{3,8}|rgba?\\(' packages/compass-viewer/src/theme.css
```

Keep literal colors only as fallbacks, graph data colors, or transparent overlays. Replace undefined aliases. Verify every `:focus-visible` style has a high-contrast equivalent.

- [ ] **Step 2: Add theme and responsive coverage**

Create `theme.spec.ts`:

```ts
for (const fixture of ["graph", "loading", "calls", "architecture", "query", "history"]) {
  test(`${fixture} uses injected light-theme tokens`, async ({ page }) => {
    await page.addInitScript(() => {
      document.documentElement.style.setProperty("--vscode-editor-background", "#f3f3f3");
      document.documentElement.style.setProperty("--vscode-editor-foreground", "#202020");
      document.documentElement.style.setProperty("--vscode-focusBorder", "#005fb8");
    });
    await page.goto(`/${fixture}.html`);
    await expect(page.locator("body")).toHaveCSS("background-color", "rgb(243, 243, 243)");
    await expect(page.locator("body")).toHaveCSS("color", "rgb(32, 32, 32)");
  });
}

test("high contrast exposes active borders", async ({ page }) => {
  await page.goto("/architecture.html");
  await page.locator("body").evaluate((body) => body.classList.add("vscode-high-contrast"));
  const width = await page.getByRole("button", { name: /Section 0/i })
    .evaluate((element) => getComputedStyle(element).borderLeftWidth);
  expect(Number.parseFloat(width)).toBeGreaterThanOrEqual(2);
});

test("all workspaces retain primary actions at 420 pixels", async ({ page }) => {
  await page.setViewportSize({ width: 420, height: 800 });
  await page.goto("/query.html");
  await expect(page.getByRole("button", { name: "Run query" })).toBeVisible();
  await page.goto("/history.html");
  await expect(page.getByRole("combobox", { name: "Select revision" })).toBeVisible();
});
```

- [ ] **Step 3: Run full viewer verification**

Run:

```bash
npm test -w @compass/viewer
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
npx playwright test -c tests/viewer/playwright.config.ts
```

Expected: all unit and Playwright tests pass with no serious or critical accessibility violations.

- [ ] **Step 4: Run full extension verification**

Run:

```bash
npm test -w editors/vscode
npm run typecheck -w editors/vscode
npm run build -w editors/vscode
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
```

Expected: tests, type check, production build, Visual Studio Extension package, and smoke check pass.

- [ ] **Step 5: Regenerate embedded and extension viewer assets**

Run:

```bash
node scripts/build_viewer_assets.mjs
npm run build -w editors/vscode
```

Expected: `viewer.css`, `graph.js`, and the asset manifest match the source tree; the extension receives the same stylesheet.

- [ ] **Step 6: Update the repository knowledge graph**

From `/Users/haipingfu/graphify`, run:

```bash
graphify update .
```

Expected: the command succeeds and refreshes the existing graph without API use.

- [ ] **Step 7: Inspect the final diff**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: no whitespace errors, no unrelated tracked files, and no generated fixtures outside existing build outputs.

- [ ] **Step 8: Commit generated assets and verification coverage**

```bash
git add tests/viewer/theme.spec.ts tests/viewer/fixtures/generate.ts tests/viewer/accessibility.spec.ts crates/compass-output/assets/viewer editors/vscode/dist/webviews/viewer.css
git commit -m "test(vscode): verify Compass workspaces across themes"
```

## Completion checklist

- [ ] Architecture loading uses the compact native shell and recovers in place
- [ ] Architecture global search groups results by subsystem
- [ ] Symbol cards search and paginate at 24 per page
- [ ] Calls filter, sort, and paginate at 25 per page
- [ ] Ask Codebase uses semantic controls and readable result tables
- [ ] Evolution automatically loads available selections
- [ ] Evolution exposes explicit build, progress, cancellation, retry, and bootstrap recovery
- [ ] Light, dark, high-contrast, reduced-motion, narrow, and large layouts pass
- [ ] Unit tests, type checks, builds, Playwright, package, and smoke checks pass
- [ ] `graphify update .` succeeds
