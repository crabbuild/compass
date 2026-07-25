# VS Code Workbench Enhancements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a polished Compass graph-loading experience, a collapsible and resizable inspector, quiet automatic CLI discovery, actionable Repository and Operations trees, and discoverable Git revision graphs.

**Architecture:** Keep graph export, validation, and command execution in the VS Code host. Add reusable inspector layout behavior to `@compass/viewer`, keep loading/error presentation in the graph webview, and derive both native VS Code trees from pure descriptor builders so their behavior can be unit tested without a VS Code process.

**Tech Stack:** TypeScript 5.9, React 19, VS Code Extension API 1.95, Zod 4, Vitest 3, Testing Library, Vite 7, esbuild 0.25, Playwright viewer tests.

## Global Constraints

- Preserve local-only processing and the existing Compass CLI capability contracts.
- Do not bundle or download the Compass CLI.
- Resolve a configured executable first, then fall back to executables named `compass` on `PATH`.
- Do not display a successfully discovered CLI path as a permanent Repository row.
- Do not build historical graphs implicitly.
- Use existing extension commands as the single execution path for tree actions.
- Preserve VS Code high-contrast themes and `prefers-reduced-motion`.
- Preserve the stacked inspector layout below `760px`.
- Run `graphify update .` from `/Users/haipingfu/graphify` after code changes.

---

### Task 1: Inspector layout state

**Files:**
- Create: `packages/compass-viewer/src/graph/inspectorLayout.ts`
- Create: `packages/compass-viewer/src/graph/inspectorLayout.test.ts`
- Modify: `packages/compass-viewer/src/index.ts`

**Interfaces:**
- Consumes: browser pointer coordinates and stored partial layout values.
- Produces: `InspectorLayout`, `DEFAULT_INSPECTOR_LAYOUT`, `normalizeInspectorLayout()`, `resizeInspectorFromPointer()`, and `resizeInspectorByKeyboard()`.

- [ ] **Step 1: Write the failing layout tests**

```ts
import { describe, expect, it } from "vitest";
import {
  DEFAULT_INSPECTOR_LAYOUT,
  normalizeInspectorLayout,
  resizeInspectorByKeyboard,
  resizeInspectorFromPointer
} from "./inspectorLayout";

describe("inspector layout", () => {
  it("normalizes stored state into the supported width range", () => {
    expect(normalizeInspectorLayout({ width: 40, collapsed: false }).width).toBe(280);
    expect(normalizeInspectorLayout({ width: 900, collapsed: true })).toEqual({
      width: 560,
      collapsed: true
    });
    expect(normalizeInspectorLayout(undefined)).toEqual(DEFAULT_INSPECTOR_LAYOUT);
  });

  it("resizes from the right-docked separator", () => {
    expect(resizeInspectorFromPointer(1200, 850)).toBe(350);
    expect(resizeInspectorFromPointer(1200, 1100)).toBe(280);
  });

  it("supports keyboard resizing in 24 pixel increments", () => {
    expect(resizeInspectorByKeyboard(340, "ArrowLeft")).toBe(364);
    expect(resizeInspectorByKeyboard(340, "ArrowRight")).toBe(316);
    expect(resizeInspectorByKeyboard(280, "ArrowRight")).toBe(280);
  });
});
```

- [ ] **Step 2: Run the focused test and confirm the missing module failure**

Run:

```bash
npm test -w @compass/viewer -- inspectorLayout.test.ts
```

Expected: FAIL because `./inspectorLayout` does not exist.

- [ ] **Step 3: Implement the pure layout model**

```ts
export const INSPECTOR_MIN_WIDTH = 280;
export const INSPECTOR_MAX_WIDTH = 560;
export const INSPECTOR_COLLAPSED_WIDTH = 48;
export const INSPECTOR_KEYBOARD_STEP = 24;

export type InspectorLayout = {
  width: number;
  collapsed: boolean;
};

export const DEFAULT_INSPECTOR_LAYOUT: InspectorLayout = {
  width: 340,
  collapsed: false
};

export function clampInspectorWidth(width: number): number {
  return Math.min(INSPECTOR_MAX_WIDTH, Math.max(INSPECTOR_MIN_WIDTH, width));
}

export function normalizeInspectorLayout(
  value: Partial<InspectorLayout> | undefined
): InspectorLayout {
  return {
    width: clampInspectorWidth(value?.width ?? DEFAULT_INSPECTOR_LAYOUT.width),
    collapsed: value?.collapsed ?? DEFAULT_INSPECTOR_LAYOUT.collapsed
  };
}

export function resizeInspectorFromPointer(containerRight: number, clientX: number): number {
  return clampInspectorWidth(containerRight - clientX);
}

export function resizeInspectorByKeyboard(width: number, key: string): number {
  if (key === "ArrowLeft") return clampInspectorWidth(width + INSPECTOR_KEYBOARD_STEP);
  if (key === "ArrowRight") return clampInspectorWidth(width - INSPECTOR_KEYBOARD_STEP);
  return clampInspectorWidth(width);
}
```

Export these symbols from `packages/compass-viewer/src/index.ts`.

- [ ] **Step 4: Run viewer unit tests and type checking**

Run:

```bash
npm test -w @compass/viewer -- inspectorLayout.test.ts
npm run typecheck -w @compass/viewer
```

Expected: both commands PASS.

- [ ] **Step 5: Commit the layout model**

```bash
git add packages/compass-viewer/src/graph/inspectorLayout.ts packages/compass-viewer/src/graph/inspectorLayout.test.ts packages/compass-viewer/src/index.ts
git commit -m "feat(viewer): add inspector layout state"
```

---

### Task 2: Collapsible and resizable graph inspector

**Files:**
- Create: `packages/compass-viewer/src/graph/InspectorResizeHandle.tsx`
- Create: `packages/compass-viewer/src/graph/InspectorResizeHandle.test.tsx`
- Modify: `packages/compass-viewer/src/graph/CompassGraph.tsx`
- Modify: `packages/compass-viewer/src/graph/GraphInspector.tsx`
- Modify: `packages/compass-viewer/src/theme.css`
- Modify: `tests/viewer/accessibility.spec.ts`
- Regenerate: `crates/compass-output/assets/viewer/graph.js`
- Regenerate: `crates/compass-output/assets/viewer/viewer.css`
- Regenerate: `crates/compass-output/assets/viewer/manifest.json`

**Interfaces:**
- Consumes: `initialInspectorLayout?: Partial<InspectorLayout>` and `onInspectorLayoutChange?: (layout: InspectorLayout) => void` on `CompassGraph`.
- Produces: a right-docked inspector with pointer resizing, arrow-key resizing, collapse/expand controls, and unchanged defaults for offline exports.

- [ ] **Step 1: Write the failing resize-handle component tests**

```tsx
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { InspectorResizeHandle } from "./InspectorResizeHandle";

describe("InspectorResizeHandle", () => {
  it("exposes separator values and keyboard resize", () => {
    const onResize = vi.fn();
    render(<InspectorResizeHandle width={340} onResize={onResize} />);
    const separator = screen.getByRole("separator", { name: "Resize graph inspector" });
    expect(separator).toHaveAttribute("aria-valuemin", "280");
    expect(separator).toHaveAttribute("aria-valuemax", "560");
    expect(separator).toHaveAttribute("aria-valuenow", "340");
    fireEvent.keyDown(separator, { key: "ArrowLeft" });
    expect(onResize).toHaveBeenCalledWith(364);
  });
});
```

- [ ] **Step 2: Run the focused test and confirm the missing component failure**

Run:

```bash
npm test -w @compass/viewer -- InspectorResizeHandle.test.tsx
```

Expected: FAIL because `InspectorResizeHandle` does not exist.

- [ ] **Step 3: Implement the accessible resize handle**

Implement `InspectorResizeHandle` with this public shape:

```tsx
export function InspectorResizeHandle({
  width,
  onResize
}: {
  width: number;
  onResize(width: number): void;
}) {
  const dragging = useRef(false);
  return (
    <div
      className="compass-inspector-resizer"
      role="separator"
      aria-label="Resize graph inspector"
      aria-orientation="vertical"
      aria-valuemin={INSPECTOR_MIN_WIDTH}
      aria-valuemax={INSPECTOR_MAX_WIDTH}
      aria-valuenow={width}
      tabIndex={0}
      onPointerDown={(event) => {
        dragging.current = true;
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (!dragging.current) return;
        const workspace = event.currentTarget.parentElement;
        if (workspace) onResize(resizeInspectorFromPointer(
          workspace.getBoundingClientRect().right,
          event.clientX
        ));
      }}
      onPointerUp={(event) => {
        dragging.current = false;
        event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onKeyDown={(event) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        event.preventDefault();
        onResize(resizeInspectorByKeyboard(width, event.key));
      }}
    />
  );
}
```

- [ ] **Step 4: Wire layout state into `CompassGraph` and `GraphInspector`**

Add the optional props without changing existing callers:

```tsx
export type CompassGraphProps = {
  model: GraphViewModel;
  host: GraphHost;
  initialInspectorLayout?: Partial<InspectorLayout>;
  onInspectorLayoutChange?(layout: InspectorLayout): void;
};
```

Normalize once when state is created, update the CSS width variable, render the
separator only while expanded, and notify the host after width or collapsed state
changes:

```tsx
const [inspectorLayout, setInspectorLayout] = useState(
  () => normalizeInspectorLayout(initialInspectorLayout)
);
const updateInspector = (next: InspectorLayout) => {
  setInspectorLayout(next);
  onInspectorLayoutChange?.(next);
};

<div
  className="compass-workspace"
  data-inspector-collapsed={inspectorLayout.collapsed}
  style={{ "--compass-inspector-width": `${inspectorLayout.width}px` } as CSSProperties}
>
  <main className="compass-graph-stage">
    <VisNetworkCanvas
      ref={canvasRef}
      model={model}
      focusedNodeId={state.focusedNodeId}
      physicsRunning={state.physicsRunning}
      forceLabels={state.forceLabels}
      hiddenCommunities={state.hiddenCommunities}
      onFocus={focus}
      onOpenSource={openNodeSource}
      onHover={setHover}
      onClear={clear}
      onStabilized={handleStabilized}
    />
    <GraphToolbar
      status={status}
      physicsRunning={state.physicsRunning}
      forceLabels={state.forceLabels}
      onTogglePhysics={() => dispatch({
        type: "setPhysics",
        running: !state.physicsRunning
      })}
      onFit={() => canvasRef.current?.fit()}
      onReset={() => {
        clear();
        canvasRef.current?.reset();
      }}
      onToggleLabels={() => dispatch({
        type: "setLabels",
        visible: !state.forceLabels
      })}
    />
    {hover && hovered && <NodeHoverCard node={hovered} hover={hover} />}
  </main>
  {!inspectorLayout.collapsed && (
    <InspectorResizeHandle
      width={inspectorLayout.width}
      onResize={(width) => updateInspector({ ...inspectorLayout, width })}
    />
  )}
  <GraphInspector
    model={model}
    selected={selected}
    neighbors={neighbors}
    query={state.query}
    matches={matches}
    hiddenCommunities={state.hiddenCommunities}
    onQueryChange={(query) => dispatch({ type: "search", query })}
    onFocus={focus}
    onOpenSource={host.openSource}
    onToggleCommunity={(communityId) => dispatch({
      type: "toggleCommunity",
      communityId
    })}
    onSetAllVisible={(visible) => dispatch({
      type: "setHiddenCommunities",
      communityIds: visible ? [] : model.communities.map((community) => community.id)
    })}
    collapsed={inspectorLayout.collapsed}
    onToggleCollapsed={() => updateInspector({
      ...inspectorLayout,
      collapsed: !inspectorLayout.collapsed
    })}
  />
</div>
```

`GraphInspector` must render a narrow rail containing an accessible `Expand graph
inspector` button when collapsed, and add a `Collapse graph inspector` button to its
existing header when expanded. Use `PanelRightCloseIcon` and
`PanelRightOpenIcon` from `lucide-react`.

- [ ] **Step 5: Add responsive, focus, high-contrast, and reduced-motion styles**

Change the desktop grid to:

```css
.compass-workspace {
  grid-template-columns: minmax(0, 1fr) 8px var(--compass-inspector-width, 340px);
}

.compass-workspace[data-inspector-collapsed="true"] {
  grid-template-columns: minmax(0, 1fr) 48px;
}

.compass-inspector-resizer {
  position: relative;
  z-index: 6;
  cursor: col-resize;
  background: var(--compass-panel);
}

.compass-inspector-resizer::after {
  content: "";
  position: absolute;
  inset: 0 3px;
  background: var(--compass-line);
}

.compass-inspector-resizer:hover::after,
.compass-inspector-resizer:focus-visible::after {
  background: var(--compass-focus);
}
```

At `max-width: 760px`, restore a single-column stacked layout, hide
`.compass-inspector-resizer`, and render the inspector expanded so existing mobile
content remains reachable. Include the new controls in the existing focus-visible
and high-contrast selector groups.

- [ ] **Step 6: Extend the viewer accessibility test**

Add assertions:

```ts
const inspector = page.getByRole("complementary", { name: "Graph inspector" });
await expect(inspector).toBeVisible();
await expect(page.getByRole("separator", { name: "Resize graph inspector" })).toBeVisible();
await page.getByRole("button", { name: "Collapse graph inspector" }).click();
await expect(page.getByRole("button", { name: "Expand graph inspector" })).toBeVisible();
```

- [ ] **Step 7: Run viewer tests and regenerate embedded assets**

Run:

```bash
npm test -w @compass/viewer
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
node scripts/build_viewer_assets.mjs
npx playwright test tests/viewer/accessibility.spec.ts tests/viewer/graph-parity.spec.ts
```

Expected: all tests PASS and the embedded viewer manifest matches the generated
JavaScript and CSS.

- [ ] **Step 8: Commit the inspector UI**

```bash
git add packages/compass-viewer/src/graph/InspectorResizeHandle.tsx packages/compass-viewer/src/graph/InspectorResizeHandle.test.tsx packages/compass-viewer/src/graph/CompassGraph.tsx packages/compass-viewer/src/graph/GraphInspector.tsx packages/compass-viewer/src/theme.css tests/viewer/accessibility.spec.ts crates/compass-output/assets/viewer/graph.js crates/compass-output/assets/viewer/viewer.css crates/compass-output/assets/viewer/manifest.json
git commit -m "feat(viewer): make graph inspector flexible"
```

---

### Task 3: Graph loading and recoverable error experience

**Files:**
- Create: `editors/vscode/src/webviews/GraphLoadingState.tsx`
- Create: `editors/vscode/src/webviews/GraphLoadingState.test.tsx`
- Modify: `editors/vscode/src/webviews/graph.tsx`
- Modify: `editors/vscode/src/transport/messages.ts`
- Modify: `editors/vscode/src/views/graphPanel.ts`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/package.json`
- Modify: `package-lock.json`
- Modify: `packages/compass-viewer/src/theme.css`
- Regenerate: `crates/compass-output/assets/viewer/viewer.css`
- Regenerate: `crates/compass-output/assets/viewer/manifest.json`

**Interfaces:**
- Consumes: host messages `hydrateGraph` and `error`; VS Code webview persistence through `getState()` and `setState()`.
- Produces: webview messages `{ type: "retry" }` and `{ type: "showOutput" }`, immediate loading UI, recoverable errors, and persisted `InspectorLayout`.

- [ ] **Step 1: Add explicit webview testing dependencies**

Run:

```bash
npm install -w editors/vscode -D @testing-library/react@^16.3.0 @testing-library/jest-dom@^6.9.1 jsdom@^27.0.0
```

Expected: `editors/vscode/package.json` and `package-lock.json` declare the testing
packages with no new runtime dependency.

- [ ] **Step 2: Write failing loading/error component tests**

```tsx
/* @vitest-environment jsdom */
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { GraphLoadingState } from "./GraphLoadingState";

describe("GraphLoadingState", () => {
  it("announces graph mapping without exposing decorative nodes", () => {
    render(<GraphLoadingState state={{ kind: "loading" }} onRetry={vi.fn()} onShowOutput={vi.fn()} />);
    expect(screen.getByRole("status")).toHaveTextContent("Mapping your codebase");
    expect(screen.getByTestId("graph-constellation")).toHaveAttribute("aria-hidden", "true");
  });

  it("offers retry and output actions after an error", () => {
    const retry = vi.fn();
    const showOutput = vi.fn();
    render(<GraphLoadingState
      state={{ kind: "error", message: "viewer export failed" }}
      onRetry={retry}
      onShowOutput={showOutput}
    />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    fireEvent.click(screen.getByRole("button", { name: "Show Compass output" }));
    expect(retry).toHaveBeenCalledOnce();
    expect(showOutput).toHaveBeenCalledOnce();
  });
});
```

- [ ] **Step 3: Run the focused tests and confirm the missing component failure**

Run:

```bash
npm test -w editors/vscode -- GraphLoadingState.test.tsx
```

Expected: FAIL because `GraphLoadingState` does not exist.

- [ ] **Step 4: Build the loading constellation and error shell**

Create the discriminated state and component:

```tsx
import { AlertTriangleIcon, CompassIcon, RotateCcwIcon, TerminalSquareIcon } from "lucide-react";

export type GraphLoadState =
  | { kind: "loading" }
  | { kind: "error"; message: string };

export function GraphLoadingState({
  state,
  onRetry,
  onShowOutput
}: {
  state: GraphLoadState;
  onRetry(): void;
  onShowOutput(): void;
}) {
  return (
    <main className="compass-load-shell">
      <div className="compass-load-constellation" data-testid="graph-constellation" aria-hidden="true">
        <span className="compass-load-orbit" />
        <span className="compass-load-node compass-load-node-a" />
        <span className="compass-load-node compass-load-node-b" />
        <span className="compass-load-node compass-load-node-c" />
        <span className="compass-load-mark">
          {state.kind === "loading" ? <CompassIcon /> : <AlertTriangleIcon />}
        </span>
      </div>
      <section role={state.kind === "loading" ? "status" : "alert"} aria-live="polite">
        <span className="compass-load-eyebrow">Compass graph</span>
        <h1>{state.kind === "loading" ? "Mapping your codebase" : "Compass could not load this graph"}</h1>
        {state.kind === "loading" ? (
          <p>Reading graph <b>·</b> Arranging relationships <b>·</b> Preparing inspector</p>
        ) : (
          <>
            <p>{state.message}</p>
            <div className="compass-load-actions">
              <button type="button" onClick={onRetry}><RotateCcwIcon />Retry</button>
              <button type="button" onClick={onShowOutput}><TerminalSquareIcon />Show Compass output</button>
            </div>
          </>
        )}
      </section>
    </main>
  );
}
```

Add the constellation, action, high-contrast, and reduced-motion styles to
`theme.css`. Use only existing Compass/VS Code color variables. The primary node
pulse must be disabled by the existing reduced-motion media query.

- [ ] **Step 5: Add typed retry/output messages and host handlers**

Extend `GraphToHostMessageSchema`:

```ts
z.object({ type: z.literal("retry") }),
z.object({ type: z.literal("showOutput") })
```

Change `GraphPanel.open()` to accept the Compass output channel:

```ts
static async open(
  context: vscode.ExtensionContext,
  session: RepositorySession,
  output: vscode.OutputChannel
): Promise<GraphPanel>
```

Handle the new messages:

```ts
if (parsed.data.type === "ready" || parsed.data.type === "retry") {
  await this.hydrate();
} else if (parsed.data.type === "showOutput") {
  this.output.show(true);
} else if (parsed.data.type === "openSource") {
  await openGraphSource(this.session, parsed.data.repositoryId, parsed.data.source);
}
```

Pass `output` from the `compass.openGraph` command. Remove the raw loading sentence
from `GraphPanel.html()` so the React webview owns all loading presentation.

- [ ] **Step 6: Persist inspector layout and render state transitions**

Use an expanded VS Code API type:

```ts
type WebviewState = { inspector?: InspectorLayout };
declare function acquireVsCodeApi(): {
  postMessage(message: unknown): void;
  getState(): WebviewState | undefined;
  setState(state: WebviewState): void;
};
```

Render loading before posting `ready`, and render it again before posting `retry`:

```tsx
const renderLoading = () => root.render(
  <GraphLoadingState
    state={{ kind: "loading" }}
    onRetry={() => {
      renderLoading();
      vscode.postMessage({ type: "retry" });
    }}
    onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
  />
);

renderLoading();
vscode.postMessage({ type: "ready" });
```

On hydration, render:

```tsx
<CompassGraph
  model={parsed.data.model}
  host={host}
  initialInspectorLayout={vscode.getState()?.inspector}
  onInspectorLayoutChange={(inspector) => vscode.setState({ inspector })}
/>
```

On `error`, render the component with the host error message.

- [ ] **Step 7: Run focused tests, build assets, and type check**

Run:

```bash
npm test -w editors/vscode -- GraphLoadingState.test.tsx
npm run typecheck -w editors/vscode
npm run build -w @compass/viewer
node scripts/build_viewer_assets.mjs
npm run build -w editors/vscode
```

Expected: all commands PASS and the VS Code bundle includes the new loading shell.

- [ ] **Step 8: Commit the loading experience**

```bash
git add editors/vscode/src/webviews/GraphLoadingState.tsx editors/vscode/src/webviews/GraphLoadingState.test.tsx editors/vscode/src/webviews/graph.tsx editors/vscode/src/transport/messages.ts editors/vscode/src/views/graphPanel.ts editors/vscode/src/extension.ts editors/vscode/package.json package-lock.json packages/compass-viewer/src/theme.css crates/compass-output/assets/viewer/viewer.css crates/compass-output/assets/viewer/manifest.json
git commit -m "feat(vscode): add polished graph loading state"
```

---

### Task 4: Automatic CLI fallback and Repository tree

**Files:**
- Create: `editors/vscode/src/views/treeModel.ts`
- Create: `editors/vscode/src/views/treeModel.test.ts`
- Modify: `editors/vscode/src/cli/discovery.ts`
- Modify: `editors/vscode/src/cli/discovery.test.ts`
- Modify: `editors/vscode/src/views/statusTree.ts`
- Modify: `editors/vscode/src/extension.ts`

**Interfaces:**
- Consumes: `CompassDiscovery` and snapshots of `RepositorySession`.
- Produces: `TreeNode` descriptors and `buildRepositoryTree(discovery, sessions)`.

- [ ] **Step 1: Write failing CLI fallback tests**

Add:

```ts
it("falls back to PATH when the configured executable is unavailable", async () => {
  const directory = path.join(process.cwd(), `.tmp-discovery-path-${Date.now()}`);
  created.push(directory);
  await mkdir(directory);
  const executable = path.join(directory, "compass");
  await writeFile(executable, "#!/bin/sh\n");
  chmodSync(executable, 0o755);
  const result = await discoverCompass(
    { get: () => path.join(directory, "missing-compass") },
    { PATH: directory },
    "darwin"
  );
  expect(result).toEqual({ kind: "found", executable });
});
```

- [ ] **Step 2: Write failing Repository model tests**

```ts
import { describe, expect, it } from "vitest";
import { buildRepositoryTree } from "./treeModel";

describe("buildRepositoryTree", () => {
  it("hides a healthy discovered CLI and exposes graph actions", () => {
    const nodes = buildRepositoryTree(
      { kind: "found", executable: "/usr/local/bin/compass" },
      [{ id: "repo", root: "/work/repo", graphState: "available", capabilityError: undefined }]
    );
    expect(nodes.map((node) => node.label)).toEqual(["repo"]);
    expect(nodes[0]?.children?.map((node) => node.command)).toEqual([
      "compass.openGraph",
      "compass.openHistory"
    ]);
  });

  it("shows setup only when CLI discovery failed", () => {
    const nodes = buildRepositoryTree(
      { kind: "missing", searched: ["/usr/bin/compass"] },
      [{ id: "repo", root: "/work/repo", graphState: "not-materialized", capabilityError: undefined }]
    );
    expect(nodes[0]).toMatchObject({
      label: "Compass CLI needs attention",
      command: "compass.selectCli"
    });
    expect(nodes[1]?.children?.[0]?.command).toBe("compass.initialize");
  });
});
```

- [ ] **Step 3: Run focused tests and confirm failures**

Run:

```bash
npm test -w editors/vscode -- discovery.test.ts treeModel.test.ts
```

Expected: the fallback assertion FAILS and `treeModel` is missing.

- [ ] **Step 4: Make discovery try configured and PATH candidates in order**

Build and deduplicate the candidates:

```ts
const pathCandidates = (environment.PATH ?? "")
  .split(path.delimiter)
  .filter(Boolean)
  .flatMap((directory) => platform === "win32"
    ? ["compass.exe", "compass.cmd", "compass.bat"].map((name) => path.join(directory, name))
    : [path.join(directory, "compass")]);
const candidates = [...new Set([...(configured ? [configured] : []), ...pathCandidates])];
```

Keep the existing executable access check and return type.

- [ ] **Step 5: Implement pure tree descriptors and the nested Repository tree**

Use:

```ts
export type TreeNode = {
  id: string;
  label: string;
  description?: string;
  tooltip?: string;
  icon: string;
  command?: string;
  children?: TreeNode[];
};

export type SessionTreeSnapshot = {
  id: string;
  root: string;
  graphState: GraphState;
  capabilityError: string | undefined;
  activeWriter?: unknown;
  watch?: unknown;
};
```

`buildRepositoryTree()` must:

- prepend one `Compass CLI needs attention` action for missing discovery;
- prepend one incompatible CLI action when any session has `capabilityError`;
- use `path.basename(root)` as the repository label and the full root as tooltip;
- add `Open graph` and `Codebase evolution` children for available graphs;
- add `Initialize repository` for non-materialized graphs;
- add `Update graph` after a failed build;
- never include a healthy executable path row.

Update `StatusTree` to use `TreeNode`, map its icon to `ThemeIcon`, map its command
to a `vscode.Command`, return `children` from `getChildren(element)`, and use
`Expanded` for repository nodes.

- [ ] **Step 6: Pass discovery into `StatusTree`**

Replace the `cliLabel` constructor argument with `CompassDiscovery`:

```ts
const statusTree = new StatusTree(registry, discovery);
```

Keep the setup notification and binary selection command unchanged.

- [ ] **Step 7: Run Repository and CLI tests**

Run:

```bash
npm test -w editors/vscode -- discovery.test.ts treeModel.test.ts
npm run typecheck -w editors/vscode
```

Expected: all tests PASS.

- [ ] **Step 8: Commit CLI and Repository improvements**

```bash
git add editors/vscode/src/views/treeModel.ts editors/vscode/src/views/treeModel.test.ts editors/vscode/src/cli/discovery.ts editors/vscode/src/cli/discovery.test.ts editors/vscode/src/views/statusTree.ts editors/vscode/src/extension.ts
git commit -m "feat(vscode): streamline repository setup"
```

---

### Task 5: Operations command center and reliable operation state

**Files:**
- Modify: `editors/vscode/src/views/treeModel.ts`
- Modify: `editors/vscode/src/views/treeModel.test.ts`
- Modify: `editors/vscode/src/views/operationsTree.ts`
- Modify: `editors/vscode/src/workspace/sessionRegistry.ts`
- Create: `editors/vscode/src/workspace/sessionRegistry.test.ts`
- Modify: `editors/vscode/src/commands/buildCommands.ts`

**Interfaces:**
- Consumes: `SessionTreeSnapshot[]`.
- Produces: `buildOperationsTree(sessions)` with Active operations, Build, Explore, and History groups.

- [ ] **Step 1: Write failing Operations model tests**

```ts
import { buildOperationsTree } from "./treeModel";

it("groups available actions and places active work first", () => {
  const nodes = buildOperationsTree([{
    id: "repo",
    root: "/work/repo",
    graphState: "available",
    capabilityError: undefined,
    activeWriter: { operationId: "build-1" },
    watch: { operationId: "watch-1" }
  }]);
  expect(nodes.map((node) => node.label)).toEqual([
    "Active operations",
    "Build",
    "Explore",
    "History"
  ]);
  expect(nodes[0]?.children?.map((node) => node.label)).toEqual([
    "Building graph",
    "Watching for changes"
  ]);
  expect(nodes[1]?.children?.map((node) => node.command)).toEqual([
    "compass.update",
    "compass.toggleWatch"
  ]);
  expect(nodes[3]?.children?.[0]?.command).toBe("compass.openHistory");
});

it("offers initialization before a graph exists", () => {
  const nodes = buildOperationsTree([{
    id: "repo",
    root: "/work/repo",
    graphState: "not-materialized",
    capabilityError: undefined
  }]);
  expect(nodes.find((node) => node.label === "Build")?.children?.map((node) => node.command))
    .toEqual(["compass.initialize"]);
});
```

- [ ] **Step 2: Write failing session refresh tests**

Extract a pure state resolver and test it:

```ts
import { describe, expect, it } from "vitest";
import { refreshedGraphState } from "./sessionRegistry";

describe("refreshedGraphState", () => {
  it("preserves active builds and failures", () => {
    expect(refreshedGraphState("available", true, true)).toBe("building");
    expect(refreshedGraphState("failed", false, false)).toBe("failed");
  });

  it("uses materialization when no operation or failure owns the state", () => {
    expect(refreshedGraphState("available", false, false)).toBe("not-materialized");
    expect(refreshedGraphState("failed", true, false)).toBe("available");
  });
});
```

- [ ] **Step 3: Run the focused tests and confirm failures**

Run:

```bash
npm test -w editors/vscode -- treeModel.test.ts sessionRegistry.test.ts
```

Expected: FAIL because the operations builder and state resolver are absent.

- [ ] **Step 4: Implement the contextual Operations descriptor builder**

Build the groups with these exact actions:

```ts
const exploreActions: TreeNode[] = [
  action("open-graph", "Open graph", "type-hierarchy", "compass.openGraph"),
  action("call-graph", "Call graph from cursor", "references", "compass.openCallGraph"),
  action("architecture", "Architecture flow", "circuit-board", "compass.openArchitecture"),
  action("query", "Query codebase", "search", "compass.openQuery")
];

const historyActions: TreeNode[] = [
  action("history", "Codebase evolution", "history", "compass.openHistory")
];
```

For an available graph, Build contains Update and Start/Stop watch. For a missing
graph, Build contains Initialize only. If any session has an active writer or watch,
prepend Active operations with spinning sync and eye icons. Include the repository
name in descriptions when more than one workspace is open.

- [ ] **Step 5: Render nested Operations nodes**

Update `OperationsTree` to use `TreeNode`, return descriptor children from
`getChildren(element)`, create VS Code commands from `node.command`, and use
`Expanded` for Active operations and `Collapsed` for command groups. Remove the
passive `No active operations` row because the available command groups now fill the
view.

- [ ] **Step 6: Preserve building and failed states during refresh**

Implement:

```ts
export function refreshedGraphState(
  current: GraphState,
  materialized: boolean,
  hasActiveWriter: boolean
): GraphState {
  if (hasActiveWriter) return "building";
  if (materialized) return "available";
  if (current === "failed") return "failed";
  return "not-materialized";
}
```

Use it in `SessionRegistry.refresh()`. In `runGuided()`, create and assign
`session.activeWriter` before calling `refresh()` so the Operations and Repository
views observe the build immediately.

- [ ] **Step 7: Run Operations and state tests**

Run:

```bash
npm test -w editors/vscode -- treeModel.test.ts sessionRegistry.test.ts
npm run typecheck -w editors/vscode
```

Expected: all tests PASS.

- [ ] **Step 8: Commit the command center**

```bash
git add editors/vscode/src/views/treeModel.ts editors/vscode/src/views/treeModel.test.ts editors/vscode/src/views/operationsTree.ts editors/vscode/src/workspace/sessionRegistry.ts editors/vscode/src/workspace/sessionRegistry.test.ts editors/vscode/src/commands/buildCommands.ts
git commit -m "feat(vscode): complete operations command center"
```

---

### Task 6: User guidance for Repository, Operations, and history

**Files:**
- Modify: `editors/vscode/README.md`
- Modify: `editors/vscode/package.json`
- Modify: `editors/vscode/src/test/suite/extension.integration.ts`

**Interfaces:**
- Consumes: existing commands and the tree structures from Tasks 4 and 5.
- Produces: discoverable view-title history action and concise end-user instructions.

- [ ] **Step 1: Write the failing command/menu integration assertion**

Extend the integration test to assert that the history command remains registered
and execute the `commands.getCommands(true)` check with:

```ts
for (const command of [
  "compass.initialize",
  "compass.update",
  "compass.toggleWatch",
  "compass.openGraph",
  "compass.openCallGraph",
  "compass.openArchitecture",
  "compass.openQuery",
  "compass.openHistory",
  "compass.selectCli"
]) {
  assert.ok(commands.has(command), `${command} is registered`);
}
```

Add a package manifest assertion that `compass.openHistory` is contributed to the
Repository view title:

```ts
const extension = vscode.extensions.getExtension("crabbuild.compass-vscode");
const menus = extension?.packageJSON.contributes.menus["view/title"] as Array<{
  command: string;
  when: string;
}>;
assert.ok(menus.some((item) =>
  item.command === "compass.openHistory" && item.when === "view == compass.status"
));
```

- [ ] **Step 2: Add the Repository title history action**

Add:

```json
{
  "command": "compass.openHistory",
  "when": "view == compass.status",
  "group": "navigation@2"
}
```

Keep Open Graph and Update in the same native view-title menu.

- [ ] **Step 3: Document how the two views work**

Add a `Using the Compass activity bar` section to `editors/vscode/README.md` with
these points:

- Repository shows one workspace row, graph state, and contextual graph/history
  actions; the CLI appears only when setup needs attention.
- Operations is the command center for Initialize, Update, Watch, Open Graph, Call
  Graph, Architecture, Query, and Codebase Evolution; active builds and watchers
  appear first.
- Codebase Evolution lists reachable commits. Select a commit, choose Build graph
  when needed, then Open graph, Compare parent, or Query this revision. Opening the
  timeline does not build revisions.

- [ ] **Step 4: Build and run the extension integration test**

Run:

```bash
npm run build -w editors/vscode
npm run test:integration -w editors/vscode
```

Expected: the test host activates the extension and confirms the history command
and Repository view-title entry.

- [ ] **Step 5: Commit user guidance**

```bash
git add editors/vscode/README.md editors/vscode/package.json editors/vscode/src/test/suite/extension.integration.ts
git commit -m "docs(vscode): explain repository operations and history"
```

---

### Task 7: Full verification and graph refresh

**Files:**
- Modify if required by generated output: `crates/compass-output/assets/viewer/graph.js`
- Modify if required by generated output: `crates/compass-output/assets/viewer/viewer.css`
- Modify if required by generated output: `crates/compass-output/assets/viewer/manifest.json`
- Refresh: `/Users/haipingfu/graphify/graphify-out/`

**Interfaces:**
- Consumes: the complete implementation from Tasks 1–6.
- Produces: verified extension artifacts, current embedded viewer assets, and an updated Graphify knowledge graph.

- [ ] **Step 1: Run all JavaScript unit tests and type checks**

Run:

```bash
npm run test:js
npm run typecheck:js
```

Expected: every workspace test and type check PASS.

- [ ] **Step 2: Rebuild and validate embedded viewer assets**

Run:

```bash
npm run build -w @compass/viewer
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
```

Expected: asset validation exits successfully with no manifest mismatch.

- [ ] **Step 3: Run viewer browser coverage**

Run:

```bash
npx playwright test tests/viewer/accessibility.spec.ts tests/viewer/graph-parity.spec.ts
```

Expected: both browser specs PASS in the configured projects.

- [ ] **Step 4: Build, package, and smoke-test the VSIX**

Run:

```bash
npm run build -w editors/vscode
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
```

Expected: packaging produces a `.vsix`; smoke validation confirms required files,
commands, and local webview assets.

- [ ] **Step 5: Inspect the final diff for generated noise and unrelated files**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only the planned source, tests, docs, package metadata, and generated
viewer assets are modified; `git diff --check` prints nothing.

- [ ] **Step 6: Refresh the root Graphify knowledge graph**

Run:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

Expected: `graphify-out/GRAPH_REPORT.md`, `graphify-out/graph.json`, labels,
manifest, and cache metadata reflect the code changes without API usage.

- [ ] **Step 7: Commit final generated artifacts if the earlier commits did not capture them**

```bash
git add crates/compass-output/assets/viewer/graph.js crates/compass-output/assets/viewer/viewer.css crates/compass-output/assets/viewer/manifest.json
git diff --cached --quiet || git commit -m "chore(viewer): refresh embedded assets"
```

Do not stage root `graphify-out/` unless that directory is already intentionally
tracked by the root repository.
