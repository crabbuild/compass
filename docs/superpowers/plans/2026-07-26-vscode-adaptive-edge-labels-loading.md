# VS Code Adaptive Edge Labels and Graph Loading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add export-parity relationship labels that appear adaptively in the VS Code graph and replace every blank graph-loading interval with a polished, truthful loading surface.

**Architecture:** Preserve the complete edge vocabulary in the Rust-to-TypeScript viewer contract, keep label formatting and visibility as pure shared-viewer helpers, and mutate vis-network edge records in place from focus, hover, zoom, and explicit label state. Keep loading work in the VS Code host, add static first-paint markup before the JavaScript bundle runs, and let the existing React loader render real snapshot/export phases with one restrained graph-constellation animation.

**Tech Stack:** Rust and Serde, TypeScript 5.9, React 19, vis-network 9, Zod 4, Vitest 3, Playwright 1.56, VS Code webviews, CSS using VS Code semantic variables.

> **Approved revision:** The user subsequently selected direct edge hover as
> the only edge-label trigger. Clicked edges, focused nodes, zoom, and `Show
> labels` do not reveal edge labels. The loader uses only the current tilted
> Compass logo, without the graph constellation. This revision supersedes the
> original disclosure and constellation steps below.

## Global Constraints

- Implement production behavior before adding its regression tests, per the user's explicit request; do not use a red-green TDD sequence.
- Visible labels must use the HTML export format: `relation [CONFIDENCE]`.
- Preserve `AGGREGATED` explicitly; do not normalize it to inferred.
- Default wide views keep edge labels hidden.
- Reveal an edge label when hovered, incident to the focused node, zoomed to the close-reading threshold, or forced by `Show labels`.
- `Show labels` controls both node and edge labels and reverts to adaptive behavior when disabled.
- Mutate the existing vis-network `DataSet`; do not recreate the `Network` for label, theme, focus, hover, or zoom changes.
- Use only bundled assets and VS Code semantic color variables; do not add dependencies, remote fonts, or remote images.
- Loading phases must describe real work and must not show a numeric percentage.
- The first-paint fallback must remain useful if the webview script never starts.
- Preserve light, dark, high-contrast, narrow-panel, keyboard, and reduced-motion behavior.
- Keep the existing graph schema version `compass.viewer.graph/1`; all contract changes are additive.
- Preserve unrelated working-tree changes and generated artifacts.
- After code changes, run `graphify update .` from `/Users/haipingfu/graphify`.

## File Structure

### New files

- `packages/compass-viewer/src/graph/edgeLabels.ts`
  - Owns edge-label formatting, the close-reading zoom threshold, and the pure adaptive visibility decision.
- `packages/compass-viewer/src/graph/edgeLabels.test.ts`
  - Covers formatting and every adaptive visibility branch after implementation.
- `editors/vscode/src/webviews/graphLoadingMarkup.ts`
  - Owns the static, accessible first-paint HTML inserted into the graph webview root.
- `editors/vscode/src/webviews/graphLoadingMarkup.test.ts`
  - Verifies the fallback is useful without React or a host response.

### Modified source files

- `crates/compass-output/src/viewer_model.rs`
  - Preserves `AGGREGATED` during viewer-model projection.
- `packages/compass-viewer/src/contracts/graph.ts`
  - Adds `aggregated` to the accepted edge-confidence values.
- `packages/compass-viewer/src/contracts/graph.test.ts`
  - Adds post-implementation contract coverage.
- `packages/compass-viewer/src/graph/networkEvents.ts`
  - Types and forwards edge-hover, edge-blur, drag, and zoom events.
- `packages/compass-viewer/src/graph/networkEvents.test.ts`
  - Adds post-implementation event adapter coverage.
- `packages/compass-viewer/src/graph/VisNetworkCanvas.tsx`
  - Applies adaptive labels and theme-aware edge-label typography without rebuilding the network.
- `packages/compass-viewer/src/graph/VisNetworkCanvas.test.ts`
  - Covers edge presentation derivation after implementation.
- `editors/vscode/src/webviews/GraphLoadingState.tsx`
  - Renders the graph constellation and completed/active/pending step states.
- `editors/vscode/src/webviews/GraphLoadingState.test.tsx`
  - Covers semantic loading phase output after implementation.
- `editors/vscode/src/webviews/graph.tsx`
  - Maps snapshotting and exporting host messages to truthful active-step copy.
- `editors/vscode/src/views/graphPanel.ts`
  - Inserts the static loader before the graph script and preserves the existing CSP.
- `packages/compass-viewer/src/theme.css`
  - Styles adaptive canvas labels indirectly through theme values and supplies the focused constellation loader.
- `tests/viewer/fixtures/generate.ts`
  - Adds aggregate confidence and large-graph phase fixtures.
- `tests/viewer/graph-parity.spec.ts`
  - Verifies label-toggle repaint behavior without network recreation.
- `tests/viewer/loading.spec.ts`
  - Verifies the new visual structure, phase state, reduced motion, and recovery behavior.
- `tests/viewer/theme.spec.ts`
  - Extends high-contrast and token-driven loader checks if existing assertions do not cover the new nodes and trace.

### Generated files

- `crates/compass-output/assets/viewer/graph.js`
- `crates/compass-output/assets/viewer/viewer.css`
- `crates/compass-output/assets/viewer/manifest.json`

Regenerate these only with `node scripts/build_viewer_assets.mjs`. VS Code `dist/`
and viewer `dist/` directories are ignored build outputs used for tests and
packaging; do not stage them.

---

### Task 1: Preserve edge semantics and centralize label rules

**Files:**
- Create: `packages/compass-viewer/src/graph/edgeLabels.ts`
- Create: `packages/compass-viewer/src/graph/edgeLabels.test.ts`
- Modify: `crates/compass-output/src/viewer_model.rs`
- Modify: `packages/compass-viewer/src/contracts/graph.ts`
- Modify: `packages/compass-viewer/src/contracts/graph.test.ts`

**Interfaces:**
- Consumes: `GraphEdge` from `packages/compass-viewer/src/contracts/graph.ts`.
- Produces: `EDGE_LABEL_ZOOM_THRESHOLD`, `formatGraphEdgeLabel(edge: Pick<GraphEdge, "relation" | "confidence">): string`, `EdgeLabelVisibility`, and `shouldShowGraphEdgeLabel(edge, visibility): boolean`.
- Produces: serialized confidence values `"extracted" | "inferred" | "ambiguous" | "aggregated"` from Rust.

- [ ] **Step 1: Extend the shared graph contract and Rust projection**

In `packages/compass-viewer/src/contracts/graph.ts`, replace the confidence enum
with:

```ts
confidence: z.enum([
  "extracted",
  "inferred",
  "ambiguous",
  "aggregated"
]).optional()
```

In `crates/compass-output/src/viewer_model.rs`, make the confidence normalization
exhaustive for Compass confidence strings:

```rust
confidence: string(object, "confidence").map(|value| {
    match value.to_ascii_lowercase().as_str() {
        "extracted" => "extracted",
        "ambiguous" => "ambiguous",
        "aggregated" => "aggregated",
        _ => "inferred",
    }
    .to_owned()
}),
```

Do not change `GraphViewEdge.confidence` from `Option<String>` and do not change
the schema version.

- [ ] **Step 2: Implement the pure edge-label module**

Create `packages/compass-viewer/src/graph/edgeLabels.ts`:

```ts
import type { GraphEdge } from "../contracts/graph";

export const EDGE_LABEL_ZOOM_THRESHOLD = 1.1;

export type EdgeLabelVisibility = {
  forceLabels: boolean;
  focusedNodeId: string | null;
  hoveredEdgeId: string | null;
  zoomScale: number;
};

export function formatGraphEdgeLabel(
  edge: Pick<GraphEdge, "relation" | "confidence">
): string {
  const relation = edge.relation.trim();
  const confidence = edge.confidence?.trim().toLocaleUpperCase();
  if (relation && confidence) return `${relation} [${confidence}]`;
  if (relation) return relation;
  return confidence ? `[${confidence}]` : "";
}

export function shouldShowGraphEdgeLabel(
  edge: Pick<GraphEdge, "id" | "source" | "target">,
  visibility: EdgeLabelVisibility
): boolean {
  return visibility.forceLabels
    || visibility.hoveredEdgeId === edge.id
    || visibility.focusedNodeId === edge.source
    || visibility.focusedNodeId === edge.target
    || visibility.zoomScale >= EDGE_LABEL_ZOOM_THRESHOLD;
}
```

The formatter intentionally preserves relation spelling and uppercases only
confidence. The visibility helper does not inspect hidden communities; hidden
edges remain hidden through the existing `DataSet` update.

- [ ] **Step 3: Add regression tests after the implementation**

Append a contract case to
`packages/compass-viewer/src/contracts/graph.test.ts` that parses:

```ts
{
  schema: "compass.viewer.graph/1",
  title: "Aggregate",
  stats: { nodes: 2, edges: 1, communities: 2, aggregated: true },
  nodes: [
    { id: "0", label: "Core", community: 0 },
    { id: "1", label: "Data", community: 1 }
  ],
  edges: [{
    id: "aggregate-edge",
    source: "0",
    target: "1",
    relation: "2 cross-community edges",
    confidence: "aggregated"
  }],
  communities: [
    { id: 0, label: "Core", color: "#4E79A7" },
    { id: 1, label: "Data", color: "#F28E2B" }
  ]
}
```

Assert that parsed confidence equals `"aggregated"`.

Create `packages/compass-viewer/src/graph/edgeLabels.test.ts` with table-driven
format cases for:

```ts
[
  [{ relation: "contains", confidence: "extracted" }, "contains [EXTRACTED]"],
  [{ relation: "calls", confidence: "inferred" }, "calls [INFERRED]"],
  [{ relation: "references", confidence: "ambiguous" }, "references [AMBIGUOUS]"],
  [{
    relation: "2 cross-community edges",
    confidence: "aggregated"
  }, "2 cross-community edges [AGGREGATED]"],
  [{ relation: "contains" }, "contains"],
  [{ relation: "", confidence: "extracted" }, "[EXTRACTED]"],
  [{ relation: "" }, ""]
]
```

Use one edge `{ id: "e1", source: "a", target: "b" }` to assert that visibility
is false at scale `1`, true for forced labels, true for hover ID `e1`, true for
focused endpoint `a`, true for focused endpoint `b`, and true at
`EDGE_LABEL_ZOOM_THRESHOLD`.

Add a Rust test in the existing `#[cfg(test)]` module of
`crates/compass-output/src/viewer_model.rs`. Build a two-node document with one
edge carrying `"confidence": "AGGREGATED"` and assert:

```rust
assert_eq!(model.edges[0].relation, "2 cross-community edges");
assert_eq!(model.edges[0].confidence.as_deref(), Some("aggregated"));
```

- [ ] **Step 4: Run focused contract and helper tests**

Run:

```bash
cargo test -p compass-output viewer_model --locked
npm run test -w @compass/viewer -- src/contracts/graph.test.ts src/graph/edgeLabels.test.ts
npm run typecheck -w @compass/viewer
```

Expected: Rust viewer-model tests pass, both Vitest files pass, and TypeScript
reports no errors.

- [ ] **Step 5: Commit the semantic contract slice**

```bash
git add \
  crates/compass-output/src/viewer_model.rs \
  packages/compass-viewer/src/contracts/graph.ts \
  packages/compass-viewer/src/contracts/graph.test.ts \
  packages/compass-viewer/src/graph/edgeLabels.ts \
  packages/compass-viewer/src/graph/edgeLabels.test.ts
git commit -m "feat(viewer): preserve adaptive edge label semantics"
```

---

### Task 2: Render adaptive labels without rebuilding the graph

**Files:**
- Modify: `packages/compass-viewer/src/graph/networkEvents.ts`
- Modify: `packages/compass-viewer/src/graph/networkEvents.test.ts`
- Modify: `packages/compass-viewer/src/graph/VisNetworkCanvas.tsx`
- Modify: `packages/compass-viewer/src/graph/VisNetworkCanvas.test.ts`

**Interfaces:**
- Consumes: `formatGraphEdgeLabel`, `shouldShowGraphEdgeLabel`, and `EdgeLabelVisibility` from Task 1.
- Produces: `GraphNetworkEvent.scale?: number`, `GraphNetworkEvent.edges`, `GraphNetworkEvent.edge`, plus `onHoverEdge(edgeId)`, `onBlurEdge()`, and `onZoom(scale)` handlers.
- Preserves: the existing `GraphCanvasHandle`, `CompassGraph` props, `Network` instance, viewport, physics state, hidden filters, comparison styling, and node hover cards.

- [ ] **Step 1: Extend the typed network event adapter**

Update `GraphNetworkEvent`:

```ts
export type GraphNetworkEvent = {
  nodes: Array<string | number>;
  edges: Array<string | number>;
  node?: string | number;
  edge?: string | number;
  scale?: number;
  pointer: { DOM: { x: number; y: number } };
};
```

Update `GraphNetworkHandlers`:

```ts
onHoverEdge(edgeId: string): void;
onBlurEdge(): void;
onZoom(scale: number): void;
```

Keep existing node callbacks. Add these bindings:

```ts
network.on("hoverEdge", (parameters) => {
  if (parameters.edge !== undefined) {
    handlers.onHoverEdge(String(parameters.edge));
  }
});
network.on("blurEdge", () => handlers.onBlurEdge());
network.on("dragStart", () => {
  handlers.onHover(null);
  handlers.onBlurEdge();
});
network.on("zoom", (parameters) => {
  handlers.onHover(null);
  handlers.onBlurEdge();
  if (parameters.scale !== undefined) handlers.onZoom(parameters.scale);
});
```

The click and double-click behaviors remain node-specific.

- [ ] **Step 2: Add adaptive canvas state and presentation**

In `VisNetworkCanvas.tsx`:

1. Import `useCallback` and the Task 1 helpers.
2. Add `hoveredEdgeId` and `zoomScale` state:

```ts
const [hoveredEdgeId, setHoveredEdgeId] = useState<string | null>(null);
const [zoomScale, setZoomScale] = useState(0);
```

3. Resolve these theme colors alongside existing node and edge colors:

```ts
const edgeLabelColor = useMemo(
  () => cssColor("--vscode-editor-foreground", "#eef5ff"),
  [themeRevision]
);
const edgeLabelMutedColor = useMemo(
  () => cssColor("--vscode-descriptionForeground", "#9aa7b7"),
  [themeRevision]
);
const edgeLabelBackground = useMemo(
  () => cssColor("--vscode-editor-background", "#08111f"),
  [themeRevision]
);
```

4. In initial `edgeData`, retain `title` and initialize `label` as an empty
string. Replace the tooltip template with `formatGraphEdgeLabel(edge)` so
tooltips and visible labels cannot diverge:

```ts
const formatted = formatGraphEdgeLabel(edge);
return {
  id: edge.id,
  from: edge.source,
  to: edge.target,
  label: "",
  title: formatted,
  // existing appearance properties
};
```

5. Create stable callbacks before the Network effect:

```ts
const handleHoverEdge = useCallback((edgeId: string) => {
  setHoveredEdgeId(edgeId);
}, []);
const handleBlurEdge = useCallback(() => {
  setHoveredEdgeId(null);
}, []);
const handleZoom = useCallback((scale: number) => {
  setZoomScale(scale);
}, []);
```

Pass them to `bindGraphNetworkEvents`.

6. When stabilization completes, set the actual scale before invoking the
existing callback:

```ts
setZoomScale(network.getScale());
```

7. Add one effect that updates every rendered edge in place:

```ts
useEffect(() => {
  const visibility = {
    forceLabels,
    focusedNodeId,
    hoveredEdgeId,
    zoomScale
  };
  edgeData.update(model.edges.map((edge) => {
    const focused = focusedNodeId === edge.source || focusedNodeId === edge.target;
    const hovered = hoveredEdgeId === edge.id;
    return {
      id: edge.id,
      label: shouldShowGraphEdgeLabel(edge, visibility)
        ? formatGraphEdgeLabel(edge)
        : "",
      font: {
        align: "middle",
        face: "var(--vscode-font-family, system-ui)",
        size: 11,
        color: focused || hovered ? edgeLabelColor : edgeLabelMutedColor,
        background: edgeLabelBackground,
        strokeWidth: 0
      }
    };
  }));
}, [
  edgeData,
  edgeLabelBackground,
  edgeLabelColor,
  edgeLabelMutedColor,
  focusedNodeId,
  forceLabels,
  hoveredEdgeId,
  model.edges,
  zoomScale
]);
```

Use `face: "system-ui"` if vis-network does not accept the CSS variable string;
do not query or import a remote font.

Do not add any of the transient label state to the Network-construction effect's
dependencies. Confirm the effect still depends only on the stable `DataSet`
instances, comparison mode, and stable event callbacks.

- [ ] **Step 3: Add event and presentation regression tests**

Extend `networkEvents.test.ts` with a small event source that records callbacks
by event name. Emit:

- `hoverEdge` with `edge: 7` and expect `"7"`;
- `blurEdge` and expect the edge-clear callback;
- `zoom` with `scale: 1.25` and expect node hover clear, edge hover clear, and
  scale `1.25`; and
- `dragStart` and expect both hover states cleared.

In `VisNetworkCanvas.test.ts`, import the Task 1 formatter/visibility helper only
if the current test harness cannot construct vis-network safely. Assert the
actual presentation inputs used by the canvas:

```ts
expect(formatGraphEdgeLabel({
  id: "e",
  source: "a",
  target: "b",
  relation: "calls",
  confidence: "extracted"
})).toBe("calls [EXTRACTED]");
```

Also assert that the same edge is visible for focused endpoint `a` and hidden
for unrelated focus `c` at scale `1`. Do not mock the implementation module.

- [ ] **Step 4: Run viewer tests, type checking, and production build**

Run:

```bash
npm run test -w @compass/viewer -- \
  src/graph/networkEvents.test.ts \
  src/graph/VisNetworkCanvas.test.ts \
  src/graph/edgeLabels.test.ts
npm run typecheck -w @compass/viewer
npm run build:viewer
```

Expected: all focused tests pass, type checking exits zero, and Vite produces
`packages/compass-viewer/dist/graph.js` and `viewer.css`.

- [ ] **Step 5: Commit the adaptive canvas slice**

```bash
git add \
  packages/compass-viewer/src/graph/networkEvents.ts \
  packages/compass-viewer/src/graph/networkEvents.test.ts \
  packages/compass-viewer/src/graph/VisNetworkCanvas.tsx \
  packages/compass-viewer/src/graph/VisNetworkCanvas.test.ts
git commit -m "feat(viewer): reveal relationship labels adaptively"
```

---

### Task 3: Eliminate blank loading and add truthful graph-phase animation

**Files:**
- Create: `editors/vscode/src/webviews/graphLoadingMarkup.ts`
- Create: `editors/vscode/src/webviews/graphLoadingMarkup.test.ts`
- Modify: `editors/vscode/src/webviews/GraphLoadingState.tsx`
- Modify: `editors/vscode/src/webviews/GraphLoadingState.test.tsx`
- Modify: `editors/vscode/src/webviews/graph.tsx`
- Modify: `editors/vscode/src/views/graphPanel.ts`
- Modify: `packages/compass-viewer/src/theme.css`

**Interfaces:**
- Consumes: existing `graphLoadStatus` messages with `phase: "snapshotting" | "exporting"`.
- Produces: `GraphLoadingCopy.activeStep?: number`.
- Produces: `graphStaticLoadingMarkup(): string`, a trusted constant-only HTML fragment.
- Preserves: `Retry`, `Show Compass output`, `variant="architecture"`, CSP, no remote assets, and the current 8 MiB large-graph threshold.

- [ ] **Step 1: Implement completed, active, and pending loading steps**

Extend `GraphLoadingCopy`:

```ts
export type GraphLoadingCopy = {
  eyebrow: string;
  title: string;
  steps: readonly string[];
  activeStep?: number;
};
```

Set `DEFAULT_LOADING_COPY.activeStep` to `0`. Render each step as:

```tsx
const stepState = index < (loadingCopy.activeStep ?? 0)
  ? "complete"
  : index === (loadingCopy.activeStep ?? 0)
    ? "active"
    : "pending";

<span className="compass-load-step" data-state={stepState}>
  <i aria-hidden="true" />
  {step}
</span>
```

Remove the dot-only `<b>` separators. The state marker is decorative; the
human-readable step text remains inside the live status.

- [ ] **Step 2: Add the signature constellation markup**

Inside `.compass-load-constellation`, before the mark, render:

```tsx
{loading && (
  <svg className="compass-load-graph" viewBox="0 0 180 112">
    <path className="compass-load-edge compass-load-edge-a" d="M18 74 58 28 90 56" />
    <path className="compass-load-edge compass-load-edge-b" d="M90 56 132 20 162 62" />
    <path className="compass-load-edge compass-load-edge-c" d="M42 94 90 56 138 94" />
    <circle className="compass-load-node compass-load-node-a" cx="18" cy="74" r="4" />
    <circle className="compass-load-node compass-load-node-b" cx="58" cy="28" r="4" />
    <circle className="compass-load-node compass-load-node-c" cx="132" cy="20" r="4" />
    <circle className="compass-load-node compass-load-node-d" cx="162" cy="62" r="4" />
    <circle className="compass-load-node compass-load-node-e" cx="42" cy="94" r="4" />
    <circle className="compass-load-node compass-load-node-f" cx="138" cy="94" r="4" />
    <circle className="compass-load-tracer" r="3">
      <animateMotion dur="2.8s" repeatCount="indefinite"
        path="M18 74 58 28 90 56 132 20 162 62" />
    </circle>
  </svg>
)}
```

Keep the entire constellation `aria-hidden="true"`. Keep the existing Compass
or error icon at the visual center. Retain the slim progress line beneath the
constellation as a familiar VS Code progress affordance.

- [ ] **Step 3: Map real host phases to active steps**

In `graph.tsx`, produce:

```ts
loadingCopy = parsed.data.phase === "snapshotting"
  ? {
      eyebrow: `Compass code graph · ${formatBytes(parsed.data.graphBytes)}`,
      title: "Preparing a large code graph",
      steps: ["Securing snapshot", "Building overview", "Opening explorer"],
      activeStep: 0
    }
  : {
      eyebrow: `Compass code graph · ${formatBytes(parsed.data.graphBytes)}`,
      title: "Preparing a large code graph",
      steps: ["Snapshot ready", "Building overview", "Opening explorer"],
      activeStep: 1
    };
```

Do not add timers, estimated duration, or a hydration-complete step; React
replaces the loader when `hydrateGraph` arrives.

- [ ] **Step 4: Add static first-paint markup and insert it into GraphPanel**

Create `graphLoadingMarkup.ts`:

```ts
export function graphStaticLoadingMarkup(): string {
  return `<main class="compass-load-shell compass-load-shell-static">
  <div class="compass-load-constellation" aria-hidden="true">
    <span class="compass-load-mark">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor"
        stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"></circle>
        <polygon points="16 8 14 14 8 16 10 10 16 8"></polygon>
      </svg>
    </span>
    <span class="compass-load-progress"><i></i></span>
  </div>
  <section class="compass-load-copy" role="status" aria-live="polite">
    <span class="compass-load-eyebrow">Compass graph</span>
    <h1>Mapping your codebase</h1>
    <p class="compass-load-steps">
      <span class="compass-load-step" data-state="active"><i aria-hidden="true"></i>Reading graph</span>
      <span class="compass-load-step" data-state="pending"><i aria-hidden="true"></i>Arranging relationships</span>
      <span class="compass-load-step" data-state="pending"><i aria-hidden="true"></i>Preparing inspector</span>
    </p>
  </section>
</main>`;
}
```

This function contains no user input and therefore needs no escaping argument.
Import it in `graphPanel.ts` and replace:

```html
<div id="root"></div>
```

with:

```ts
<div id="root">${graphStaticLoadingMarkup()}</div>
```

Do not add inline style or script. Keep the current nonce and CSP unchanged.

- [ ] **Step 5: Implement the restrained VS Code-native visual treatment**

In the later “VS Code-native loading shell” section of `theme.css`, replace the
compact 64 px mark layout with these responsibilities:

- `.compass-load-constellation`: `width: min(180px, 64vw)`, `height: 112px`,
  centered and positioned relative.
- `.compass-load-graph`: absolute full-size SVG with `overflow: visible`.
- `.compass-load-edge`: one-pixel theme-derived strokes with a subtle animated
  dash offset.
- `.compass-load-node`: graph-accent fills with one restrained opacity pulse.
- `.compass-load-tracer`: progress-color fill plus a small theme-derived glow.
- `.compass-load-mark`: absolute centered 48 px widget surface with the
  existing restrained 12 px radius and no heavy shadow.
- `.compass-load-progress`: centered at the bottom and no wider than 96 px.
- `.compass-load-steps`: centered flex wrap with 10–14 px gaps.
- `.compass-load-step`: inline-flex, aligned marker and text.
- `[data-state="complete"]`: normal muted text and a filled marker.
- `[data-state="active"]`: editor foreground and progress-color marker.
- `[data-state="pending"]`: disabled/muted text and an outlined marker.

Use these semantic tokens with existing fallbacks:

```css
--vscode-editor-background
--vscode-editorWidget-background
--vscode-editor-foreground
--vscode-descriptionForeground
--vscode-disabledForeground
--vscode-widget-border
--vscode-progressBar-background
--vscode-symbolIcon-classForeground
--vscode-contrastBorder
```

Under the existing reduced-motion media query, include
`.compass-load-tracer`, `.compass-load-node`, and `.compass-load-edge`. Disable
their animation. Because SVG SMIL motion is not controlled by `animation-name`,
add:

```css
@media (prefers-reduced-motion: reduce) {
  .compass-load-tracer {
    display: none;
  }
}
```

In high-contrast selectors, give the mark, graph nodes, state markers, and
progress line explicit contrast borders or currentColor strokes. At the narrow
breakpoint, keep the constellation no wider than the viewport and allow steps
to stack.

- [ ] **Step 6: Add loading regression tests after implementation**

Extend `GraphLoadingState.test.tsx`:

- default markup contains `compass-load-graph`;
- default `Reading graph` step has `data-state="active"`;
- a large graph with `activeStep: 1` renders the first step complete, second
  active, and third pending;
- error markup contains no animated tracer;
- architecture variant still renders its skeleton.

Create `graphLoadingMarkup.test.ts` and assert that
`graphStaticLoadingMarkup()` contains:

```ts
expect(markup).toContain('role="status"');
expect(markup).toContain("Mapping your codebase");
expect(markup).toContain("Reading graph");
expect(markup).toContain("compass-load-progress");
expect(markup).not.toContain("<script");
expect(markup).not.toContain("<style");
```

Keep the existing error-action tests unchanged.

- [ ] **Step 7: Run focused extension tests, type checking, and build**

Run:

```bash
npm run test -w editors/vscode -- \
  src/webviews/GraphLoadingState.test.tsx \
  src/webviews/graphLoadingMarkup.test.ts \
  src/transport/messages.test.ts
npm run typecheck -w editors/vscode
npm run build:vscode
```

Expected: all tests and type checking pass; the extension build regenerates
ignored webview bundles and copies the current viewer CSS into
`editors/vscode/dist/webviews/viewer.css`.

- [ ] **Step 8: Commit the loading slice**

```bash
git add \
  editors/vscode/src/webviews/graphLoadingMarkup.ts \
  editors/vscode/src/webviews/graphLoadingMarkup.test.ts \
  editors/vscode/src/webviews/GraphLoadingState.tsx \
  editors/vscode/src/webviews/GraphLoadingState.test.tsx \
  editors/vscode/src/webviews/graph.tsx \
  editors/vscode/src/views/graphPanel.ts \
  packages/compass-viewer/src/theme.css
git commit -m "feat(vscode): polish large graph loading"
```

---

### Task 4: Add browser coverage, regenerate assets, and verify the extension

**Files:**
- Modify: `tests/viewer/fixtures/generate.ts`
- Modify: `tests/viewer/graph-parity.spec.ts`
- Modify: `tests/viewer/loading.spec.ts`
- Modify: `tests/viewer/theme.spec.ts` only if the new high-contrast assertions fit that suite better than `loading.spec.ts`
- Regenerate: `crates/compass-output/assets/viewer/graph.js`
- Regenerate: `crates/compass-output/assets/viewer/viewer.css`
- Regenerate: `crates/compass-output/assets/viewer/manifest.json`

**Interfaces:**
- Consumes: all Tasks 1–3.
- Produces: compiled export assets, browser regression coverage, a packaged VSIX smoke result, and an updated graphify knowledge graph.

- [ ] **Step 1: Update browser fixtures with real relationship vocabulary**

In `tests/viewer/fixtures/generate.ts`, change the community overview edge to:

```ts
edges: [{
  id: "overview-edge",
  source: "0",
  target: "1",
  relation: "2 cross-community edges",
  confidence: "aggregated"
}]
```

Update `graphLoadingHarness()` so `?large=1` continues sending
`phase: "exporting"` and add `?snapshot=1` to send
`phase: "snapshotting"`. Keep the exact `44_275_915` byte fixture so the UI
continues to render `42.2 MB`.

- [ ] **Step 2: Add browser regression coverage after fixture implementation**

In `loading.spec.ts`:

- assert `.compass-load-graph` and six `.compass-load-node` elements are
  visible in ordinary motion mode;
- assert `Securing snapshot` is active for `/loading.html?snapshot=1`;
- assert `Snapshot ready` is complete and `Building overview` is active for
  `/loading.html?large=1`;
- in reduced-motion mode, assert `.compass-load-tracer` is hidden and
  `.compass-load-progress i` has `animation-name: none`;
- retain centered layout and recovery-action tests.

In `graph-parity.spec.ts`, extend the existing canvas theme test:

1. Store the current `.vis-network` element reference.
2. Capture `canvas.toDataURL()`.
3. Click `Show labels`.
4. Poll until the canvas data URL changes.
5. Assert the `.vis-network` reference is still identical.
6. Click `Hide labels` and poll until the canvas changes again.

The exact relationship strings and adaptive branches are already covered by
unit tests; the browser test proves compiled webview integration and in-place
repaint without relying on canvas text as DOM.

Add or extend a high-contrast browser assertion so the loading mark has a
non-zero border and active step text remains visible. Avoid screenshot
goldens; theme variables are already exercised dynamically.

- [ ] **Step 3: Regenerate the checked-in standalone viewer assets**

Run:

```bash
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
```

Expected: the checker exits zero and only the three declared files under
`crates/compass-output/assets/viewer/` change.

- [ ] **Step 4: Run the complete relevant verification matrix**

Run from `/Users/haipingfu/graphify/compass`:

```bash
cargo fmt --all -- --check
cargo test -p compass-output viewer_model --locked
npm run typecheck:js
npm run test -w @compass/viewer
npm run test -w editors/vscode
npm run build:viewer
npm run build:vscode
npm run test -w @compass/viewer-tests -- \
  graph-parity.spec.ts \
  loading.spec.ts \
  theme.spec.ts \
  accessibility.spec.ts \
  performance.spec.ts
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
node scripts/check_viewer_assets.mjs
git diff --check
```

Expected:

- Rust formatting and viewer-model tests pass.
- All viewer and extension unit tests pass.
- Both TypeScript workspaces typecheck.
- Viewer and VS Code production builds complete.
- Targeted Chromium tests report zero failures, including accessibility and
  the existing under-one-second small-graph performance contract.
- VSIX packaging succeeds and the smoke script finds the graph bundle and
  current viewer CSS.
- Generated viewer assets match source.
- `git diff --check` reports no whitespace errors.

- [ ] **Step 5: Refresh the project graph after all code changes**

Run from `/Users/haipingfu/graphify`:

```bash
graphify update .
```

Expected: the command exits zero and refreshes `graphify-out/` for the current
source tree. Inspect `git status --short` afterward and preserve the user's
pre-existing `graphify/extractors/r.py`, `tests/fixtures/sample.r`, and `plans/`
changes.

- [ ] **Step 6: Inspect the final diff against the approved spec**

Run:

```bash
git status --short
git diff --stat HEAD~3
git diff --check
```

Confirm:

- `AGGREGATED` survives Rust and TypeScript projection.
- The default wide graph has no edge label.
- hover, focus, close zoom, and `Show labels` reveal labels.
- label updates do not rebuild the Network.
- static markup prevents an empty `#root`.
- React loading copy follows snapshotting and exporting.
- error actions, high contrast, narrow layout, and reduced motion remain.
- no unrelated source or generated file is staged.

- [ ] **Step 7: Commit browser coverage and generated assets**

```bash
git add \
  tests/viewer/fixtures/generate.ts \
  tests/viewer/graph-parity.spec.ts \
  tests/viewer/loading.spec.ts \
  tests/viewer/theme.spec.ts \
  crates/compass-output/assets/viewer/graph.js \
  crates/compass-output/assets/viewer/viewer.css \
  crates/compass-output/assets/viewer/manifest.json
git commit -m "test(vscode): verify edge labels and graph loading"
```

If `tests/viewer/theme.spec.ts` was not changed, omit it from `git add`. Do not
stage ignored `dist/` directories or the generated `.vsix`.

## Completion Criteria

The implementation is complete only when:

- all four tasks have independent commits;
- focused and full relevant verification commands have fresh passing output;
- the packaged VSIX contains the updated graph webview and viewer CSS;
- checked-in viewer assets match the shared viewer source;
- `graphify update .` has completed;
- the final diff contains no unrelated user-owned changes; and
- a reviewer can map every design-spec requirement to a source change or test
  in this plan.
