# VS Code Graph Canvas Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve the active `compass export html` renderer while rebuilding the VS Code React graph to match its visual language, follow VS Code themes, and open exact source locations on node double-click.

**Architecture:** Keep `html.rs` as the standalone HTML runtime and keep `viewer-json` as an independent `/1` IDE contract. Enrich the IDE model with optional presentation metadata, then split the React graph into a canvas event layer and focused toolbar, hover-card, and inspector components. Current, historical, and call graphs continue to reuse `CompassGraph`.

**Tech Stack:** Rust 1.97.1, serde, React 19, TypeScript, vis-network, Tailwind CSS 4, shadcn, Lucide, Zod, Vitest, Playwright, VS Code Extension API.

## Global Constraints

- `compass export html` must continue to use `html_document → render → page` in `crates/compass-output/src/html.rs`.
- The standalone HTML export keeps its fixed Compass dark palette and embedded community drill-down.
- `compass export viewer-json` remains `compass.viewer.graph/1`; all new fields are optional and backward-compatible.
- VS Code colors use `--vscode-*` variables first and the Compass export palette as fallbacks.
- Single-click focuses; double-click opens only a non-empty file with a line or byte position.
- The existing inspector source button remains keyboard-accessible.
- Call and historical graphs inherit the shared React behavior.
- Do not use a TDD workflow; implement each task, then add and run its regression coverage.
- Run this branch's Compass CLI, not Graphify, after code changes.

---

### Task 1: Restore the active standalone HTML exporter

**Files:**
- Modify: `crates/compass-output/src/html.rs`
- Test: `crates/compass-output/src/html.rs`

**Interfaces:**
- Consumes: `HtmlOptions`, `GraphDocument`, `Communities`, `python_json_compact`.
- Produces: unchanged `html_document(...) -> Result<Option<HtmlRender>, OutputError>` and independent `graph_view_model_document(...) -> Result<Option<GraphViewModel>, OutputError>`.

- [ ] **Step 1: Restore the HTML rendering route**

Restore `html_document` to apply the node limit, aggregate when explicitly requested, and call:

```rust
render(document, communities, output_path.as_ref(), options, drilldown)
```

Restore `render` from `origin/main` so it serializes nodes, edges, legend, hyperedges, and community details before calling `page`. Keep `graph_view_model_document` as a separate function used only by `viewer-json` and historical IDE exports.

- [ ] **Step 2: Restore active helper status**

Import both:

```rust
use crate::json::python_json_compact;
use crate::viewer_model::GraphViewModel;
```

Remove dead-code annotations from `community_details`, `js_safe`, and `page` because the restored route uses them.

- [ ] **Step 3: Restore export regression assertions**

Update the existing `html.rs` tests to assert these active markers:

```text
id="graph-toolbar"
id="sidebar"
class="node-hover-card"
const COMMUNITY_DETAILS =
function enterCommunity(community, focusId = null)
network.on('doubleClick'
```

Keep script-injection, accessibility, responsive, learning-overlay, and aggregated-rendering assertions.

- [ ] **Step 4: Verify the Rust export surface**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-output html::tests --locked
cargo test -p compass-cli --test viewer_export_cli --locked
```

Expected: all selected tests pass and generated HTML contains the active `html.rs` UI rather than `compass-viewer-root`.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-output/src/html.rs
git commit -m "fix(output): preserve Compass HTML graph experience"
```

### Task 2: Enrich the versioned IDE graph model

**Files:**
- Modify: `crates/compass-output/src/viewer_model.rs`
- Modify: `packages/compass-viewer/src/contracts/graph.ts`
- Modify: `packages/compass-viewer/src/contracts/graph.test.ts`
- Test: `crates/compass-output/src/viewer_model.rs`

**Interfaces:**
- Produces optional `GraphViewNode` fields serialized as `language`, `signature`, `size`, `memberCount`, `learningStatus`, and `learningStale`.
- Produces TypeScript `GraphNode` with the same optional camelCase fields.

- [ ] **Step 1: Add optional Rust presentation fields**

Extend `GraphViewNode`:

```rust
pub language: Option<String>,
pub signature: Option<String>,
pub size: Option<f64>,
pub member_count: Option<usize>,
```

Populate them from the sanitized `node_values` object. Preserve the existing optional source, kind, color, degree, and learning fields.

- [ ] **Step 2: Extend the Zod contract**

Add:

```ts
language: z.string().optional(),
signature: z.string().optional(),
size: z.number().positive().optional(),
memberCount: z.number().int().nonnegative().optional(),
learningStatus: z.string().optional(),
learningStale: z.boolean().optional()
```

Keep `.passthrough()` so older and future `/1` payloads remain compatible.

- [ ] **Step 3: Add post-implementation compatibility coverage**

Add contract cases proving a minimal old `/1` node still parses and a metadata-rich node preserves every optional field. Add a Rust model test using a function node with language, signature, line range, learning state, and aggregate member count.

- [ ] **Step 4: Verify the model**

Run:

```bash
cargo test -p compass-output viewer_model --locked
npm run typecheck -w @compass/viewer
npm run test -w @compass/viewer
```

Expected: Rust model tests and viewer contract tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-output/src/viewer_model.rs packages/compass-viewer/src/contracts/graph.ts packages/compass-viewer/src/contracts/graph.test.ts
git commit -m "feat(viewer): expose graph presentation metadata"
```

### Task 3: Add source eligibility and canvas interaction events

**Files:**
- Create: `packages/compass-viewer/src/graph/sourceNavigation.ts`
- Create: `packages/compass-viewer/src/graph/sourceNavigation.test.ts`
- Modify: `packages/compass-viewer/src/graph/VisNetworkCanvas.tsx`
- Modify: `packages/compass-viewer/src/graph/CompassGraph.tsx`

**Interfaces:**
- Produces: `navigableSource(node: GraphNode): SourceLocation | undefined`.
- Extends canvas props with `onOpenSource(nodeId: string): void` and `onHover(change: { nodeId: string; x: number; y: number } | null): void`.

- [ ] **Step 1: Implement source eligibility**

Implement:

```ts
export function navigableSource(node: GraphNode): SourceLocation | undefined {
  const source = node.source;
  if (!source?.file.trim()) return undefined;
  const located = source.startLine !== undefined
    || source.endLine !== undefined
    || source.startByte !== undefined
    || source.endByte !== undefined;
  return located ? source : undefined;
}
```

- [ ] **Step 2: Add vis-network events**

Keep `click` mapped to `onFocus`. Add `doubleClick` to emit the first selected node ID through `onOpenSource`. Add `hoverNode`, `blurNode`, `dragStart`, and `zoom` callbacks using `parameters.pointer.DOM` for hover-card positioning.

- [ ] **Step 3: Connect source navigation**

In `CompassGraph`, resolve the node ID, call `navigableSource`, and invoke `host.openSource` only when it returns a location. Keep the inspector source button on the same eligibility helper.

- [ ] **Step 4: Add post-implementation tests**

Cover file plus line, file plus byte range, whitespace-only file, file-only source, and missing source. Assert that the eligible cases return the original range and ineligible cases return `undefined`.

- [ ] **Step 5: Verify interaction types and tests**

Run:

```bash
npm run typecheck -w @compass/viewer
npm run test -w @compass/viewer
```

Expected: TypeScript passes and source navigation cases pass.

- [ ] **Step 6: Commit**

```bash
git add packages/compass-viewer/src/graph/sourceNavigation.ts packages/compass-viewer/src/graph/sourceNavigation.test.ts packages/compass-viewer/src/graph/VisNetworkCanvas.tsx packages/compass-viewer/src/graph/CompassGraph.tsx
git commit -m "feat(viewer): open graph nodes from double click"
```

### Task 4: Rewrite the React canvas with Compass export styling

**Files:**
- Create: `packages/compass-viewer/src/graph/GraphToolbar.tsx`
- Create: `packages/compass-viewer/src/graph/GraphInspector.tsx`
- Create: `packages/compass-viewer/src/graph/NodeHoverCard.tsx`
- Modify: `packages/compass-viewer/src/graph/CompassGraph.tsx`
- Modify: `packages/compass-viewer/src/graph/VisNetworkCanvas.tsx`
- Modify: `packages/compass-viewer/src/theme.css`

**Interfaces:**
- `GraphToolbar` consumes layout state and callbacks only.
- `GraphInspector` consumes the model, selected node, neighbors, visibility state, and focus/open callbacks.
- `NodeHoverCard` consumes a graph node plus viewport-relative `x` and `y`.

- [ ] **Step 1: Match the active export's network tuning**

Use:

```ts
physics: {
  solver: "forceAtlas2Based",
  forceAtlas2Based: {
    gravitationalConstant: -60,
    centralGravity: 0.005,
    springLength: 120,
    springConstant: 0.08,
    damping: 0.4,
    avoidOverlap: 0.8
  },
  stabilization: { iterations: 200, fit: true }
},
interaction: {
  hover: true,
  tooltipDelay: 100,
  hideEdgesOnDrag: true,
  navigationButtons: false,
  keyboard: { enabled: true }
},
edges: {
  smooth: { enabled: true, type: "continuous", roundness: 0.2 },
  selectionWidth: 3
}
```

Use model `size` when present and degree fallback otherwise. Preserve evidence dashes and ambiguity widths.

- [ ] **Step 2: Extract the floating toolbar**

Build `GraphToolbar` with status dot/text, pause/resume, fit, reset, and label controls using Lucide icons and accessible names. Search moves into the inspector to match the HTML export.

- [ ] **Step 3: Build the richer inspector**

Build `GraphInspector` with:

- Compass product header;
- keyboard-operable search results;
- node identity, kind, community, degree, language, lines, file, and signature;
- source-open button only for navigable nodes;
- connected-node buttons;
- select-all and per-community visibility controls; and
- graph node/edge/community statistics.

- [ ] **Step 4: Build the hover card**

Render `NodeHoverCard` over the canvas with clamped positioning. Show only present metadata and never inject HTML. Hide it during drag, zoom, focus changes, and pointer exit.

- [ ] **Step 5: Rewrite theme styles**

Define semantic variables with VS Code variables first:

```css
--compass-canvas: var(--vscode-editor-background, #08111f);
--compass-canvas-deep: color-mix(in srgb, var(--compass-canvas) 82%, #000);
--compass-panel: var(--vscode-sideBar-background, #101b2d);
--compass-panel-raised: color-mix(in srgb, var(--vscode-menu-background, #142236) 88%, transparent);
--compass-line: var(--vscode-panel-border, rgba(154, 178, 211, .16));
--compass-focus: var(--vscode-focusBorder, #76b7ff);
```

Recreate radial gradients, dotted texture, glass toolbar, 340px sidebar, metadata cards, hover card, node swatch, community rows, narrow layout, reduced motion, and high-contrast borders.

- [ ] **Step 6: Verify viewer compilation**

Run:

```bash
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
```

Expected: the viewer builds with no TypeScript or CSS errors.

- [ ] **Step 7: Commit**

```bash
git add packages/compass-viewer/src/graph packages/compass-viewer/src/theme.css
git commit -m "feat(viewer): match Compass export graph canvas"
```

### Task 5: Add browser and extension regression coverage

**Files:**
- Modify: `tests/viewer/graph-parity.spec.ts`
- Modify: `tests/viewer/accessibility.spec.ts`
- Modify: `tests/viewer/fixtures/generate.ts`

**Interfaces:**
- Consumes the shared viewer fixture and production webview bundle.
- Produces assertions for visual structure, theme adaptation, source eligibility, and responsive behavior.

- [ ] **Step 1: Enrich the graph fixture**

Include nodes with language, signature, line ranges, file-only source, and no source. Keep deterministic IDs and community colors.

- [ ] **Step 2: Add post-implementation browser checks**

Assert the rendered graph contains:

```text
Compass product header
floating graph toolbar
graph inspector
search
metadata cards
community controls
graph statistics
```

Select a source node and verify the inspector shows its file and lines. Verify file-only nodes do not expose source navigation.

- [ ] **Step 3: Check theme and responsive behavior**

Run fixtures with dark and light CSS variables and assert computed canvas/panel colors differ. At 320 CSS pixels, assert the inspector moves below the canvas and all primary controls remain reachable. Retain reduced-motion and axe checks.

- [ ] **Step 4: Run JS qualification**

Run:

```bash
npm run typecheck:js
npm run test:js
npm run test:integration -w compass-vscode
```

Expected: all unit, Chromium, and real VS Code tests pass.

- [ ] **Step 5: Commit**

```bash
git add tests/viewer editors/vscode/src/test
git commit -m "test(viewer): qualify Compass graph parity"
```

### Task 6: Document, refresh with Compass, package, and qualify

**Files:**
- Modify: `editors/vscode/README.md`
- Modify: `editors/vscode/CHANGELOG.md`
- Modify: `docs/guides/vscode.md`
- Regenerate: `crates/compass-output/assets/viewer/graph.js`
- Regenerate: `crates/compass-output/assets/viewer/viewer.css`
- Regenerate: `crates/compass-output/assets/viewer/manifest.json`

**Interfaces:**
- Produces the final local-only VSIX and deterministic viewer assets.

- [ ] **Step 1: Update user documentation**

Document VS Code theme adaptation, Compass export visual parity, single-click inspection, double-click source opening, and behavior for nodes without locations.

- [ ] **Step 2: Build deterministic assets**

Run:

```bash
npm run build:viewer
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
```

Expected: embedded viewer assets and manifest match a clean rebuild. These assets serve IDE/history consumers; the active HTML export remains the `html.rs` page.

- [ ] **Step 3: Refresh the Compass graph**

Run:

```bash
cargo build -p compass-cli --bin compass
target/debug/compass capabilities --format json
target/debug/compass update . --no-viz
```

Expected: the branch CLI advertises `compass.viewer.graph/1` and updates `compass-out`.

- [ ] **Step 4: Run final Rust and JS verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p compass-cli -p compass-output --all-targets -- -D warnings
cargo test -p compass-output -p compass-cli --locked
npm run typecheck:js
npm run test:js
npm run test:integration -w compass-vscode
node scripts/check_viewer_assets.mjs
```

Expected: every command exits zero.

- [ ] **Step 5: Package and smoke-check the VSIX**

Run:

```bash
npm run package -w compass-vscode
npm run smoke:vsix -w compass-vscode
shasum -a 256 editors/vscode/compass-vscode-0.1.0.vsix
```

Expected: VSIX contains only runtime assets and the smoke check passes.

- [ ] **Step 6: Commit**

```bash
git add editors/vscode/README.md editors/vscode/CHANGELOG.md docs/guides/vscode.md crates/compass-output/assets/viewer
git commit -m "docs(vscode): describe graph source navigation"
```

- [ ] **Step 7: Push the updated branch**

```bash
git push origin feature/compass-vscode-extension
```

Expected: draft PR #33 updates to include the implementation and verification commits.
