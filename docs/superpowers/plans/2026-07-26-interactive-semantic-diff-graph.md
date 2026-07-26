# Interactive Semantic-Diff Graph Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. This plan intentionally follows the user-approved implementation-first order: production behavior first, regression coverage immediately afterward, then a commit.

**Goal:** Replace the standalone semantic-diff report's crowded static graph with a readable, selectable changed-subgraph explorer and persistent node inspector.

**Architecture:** Keep the report deterministic, bounded, offline, and dependency-free. Extract the graph-specific CSS and JavaScript into first-party assets that Rust embeds inline, then progressively add capsule layout, selection, inspector navigation, accessibility, and responsive behavior. The existing exhaustive node/edge lists remain the non-JavaScript fallback.

**Tech Stack:** Rust 2024, embedded first-party CSS and browser JavaScript, SVG and semantic HTML, Playwright Chromium, existing Compass semantic-diff report schema.

## Global Constraints

- Scope is the standalone `compass diff --format html` report only; do not modify the VS Code `CompassGraph` experience.
- Do not change `compass.semantic_diff.report/1`, `GraphNodeDelta`, or `GraphEdgeDelta`.
- The output remains one self-contained HTML file with no remote or external runtime assets.
- Preserve deterministic node/edge ordering, bounded visual sampling, and exhaustive lists.
- Use text-only DOM assignment for report-derived content; never use `innerHTML` with graph data.
- Show only retained facts: status, label, ID, kind, source file, changed-field names, relationships, related findings, and valid source-patch targets.
- Use the existing report colors: accent `#8ab4f8`, added `#65bd84`, removed `#ff7b86`, changed `#d9a441`, and context `#8d96a5`.
- Encode status with marks and border styles in addition to color.
- Enter and Space select nodes; Escape clears; reduced-motion users receive immediate transitions.
- The inspector moves below the graph below 760 pixels.
- Follow the requested implementation-first order. Add tests after each production slice and before its commit.
- After code changes, run `graphify update .`.

---

## File map

### New files

- `crates/compass-cli/assets/semantic-diff-graph.css`
  - Owns only the standalone graph explorer, capsule, edge, focus, inspector,
    fallback, and responsive styles.
- `crates/compass-cli/assets/semantic-diff-graph.js`
  - Owns graph indexing, deterministic sampling/layout, SVG rendering,
    selection, inspector rendering, navigation, keyboard behavior, and fallback.
- `tests/viewer/semantic-diff-graph.spec.ts`
  - Exercises the exact embedded graph JavaScript and CSS in Chromium.

### Modified files

- `crates/compass-cli/src/semantic_diff_render.rs`
  - Embeds the new assets, renders the graph explorer shell, supplies source/list
    anchors, and invokes the graph mount interface.
- `crates/compass-cli/tests/history_cli.rs`
  - Verifies the generated CLI report remains self-contained and contains the
    graph explorer contract.
- `tests/viewer/fixtures/generate.ts`
  - Copies the graph assets and emits a deterministic standalone interaction
    fixture using the production DOM contract.
- `docs/reference/outputs.md`
  - Documents node selection, neighborhood focus, inspector data, and fallback.
- `docs/guides/versioned-history.md`
  - Adds the graph-inspection workflow to the existing diff guide.

## Stable interfaces

The JavaScript asset exposes exactly one global:

```js
globalThis.CompassSemanticDiffGraph = Object.freeze({
  mount(options) {
    // Returns { clear(), select(nodeId), destroy() }.
  }
});
```

`mount(options)` consumes:

```ts
type MountOptions = {
  report: {
    graph_delta: GraphDelta;
    findings: SemanticFinding[];
    source_changes: SourceFileDelta[];
  };
  host: HTMLElement;
  inspector: HTMLElement;
  liveRegion: HTMLElement;
  note: HTMLElement;
};
```

The Rust renderer supplies these stable elements:

```html
<div class="graph-explorer">
  <div id="graph-canvas"
       class="graph-canvas"
       aria-label="Changed code graph"></div>
  <aside id="graph-inspector"
         class="graph-inspector"
         aria-labelledby="graph-inspector-heading"></aside>
</div>
<p id="graph-live" class="sr-only" aria-live="polite"></p>
<p id="graph-note" class="graph-note"></p>
```

Every rendered SVG node has:

```html
<g class="graph-node added"
   data-node-id="node-id"
   role="button"
   tabindex="0"
   aria-pressed="false"
   aria-label="Added function RetryWithSmallerBatch"></g>
```

The exhaustive rows expose data hooks rather than interpolating IDs into CSS:

```html
<li class="delta-row" data-graph-node-id="node-id">...</li>
<li class="delta-row"
    data-graph-edge-source="caller"
    data-graph-edge-target="target"
    data-graph-edge-relation="calls">...</li>
```

---

### Task 1: Isolate the graph assets and establish the explorer shell

**Files:**
- Create: `crates/compass-cli/assets/semantic-diff-graph.css`
- Create: `crates/compass-cli/assets/semantic-diff-graph.js`
- Modify: `crates/compass-cli/src/semantic_diff_render.rs:10`
- Modify: `crates/compass-cli/src/semantic_diff_render.rs:373-410`
- Modify: `crates/compass-cli/src/semantic_diff_render.rs:642-769`
- Modify: `crates/compass-cli/src/semantic_diff_render.rs:829-900`
- Test: `crates/compass-cli/src/semantic_diff_render.rs:1412`
- Test: `crates/compass-cli/tests/history_cli.rs:1079`

**Interfaces:**
- Consumes: existing `SemanticDiffReport.graph_delta`, `.findings`, and
  `.source_changes`.
- Produces: `CompassSemanticDiffGraph.mount(options)` and the stable explorer
  DOM contract used by all later tasks.

- [ ] **Step 1: Add first-party asset constants**

Add beside `PIERRE_DIFFS_JS`:

```rust
const SEMANTIC_DIFF_GRAPH_CSS: &str =
    include_str!("../assets/semantic-diff-graph.css");
const SEMANTIC_DIFF_GRAPH_JS: &str =
    include_str!("../assets/semantic-diff-graph.js");
```

- [ ] **Step 2: Move the current graph styles into the CSS asset**

Move the existing `.graph-summary` through `.delta-grid` graph rules out of the
large Rust raw string. Start the asset with the current behavior-preserving
rules, then add shell-only rules:

```css
.graph-explorer {
  display: grid;
  grid-template-columns: minmax(0, 7fr) minmax(280px, 3fr);
  min-height: 440px;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface-inset);
}

.graph-canvas {
  min-width: 0;
  min-height: 440px;
  border: 0;
  border-right: 1px solid var(--border);
  border-radius: 0;
}

.graph-inspector {
  min-width: 0;
  padding: 18px;
  background: var(--surface);
}

.graph-inspector-empty {
  color: var(--muted);
  font-size: 12px;
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
```

Split the renderer's `<style>` output before `</head>`, append
`SEMANTIC_DIFF_GRAPH_CSS`, then close the style element. Do not create a
stylesheet link.

- [ ] **Step 3: Move the existing SVG renderer into the JavaScript asset**

Wrap the current `renderChangedGraph` behavior in this public boundary:

```js
(() => {
  "use strict";

  function mount({ report, host, inspector, liveRegion, note }) {
    renderCurrentGraph(report.graph_delta, host, note);
    inspector.replaceChildren(
      element("p", "graph-inspector-empty", "Select a node to inspect its change.")
    );
    return Object.freeze({
      clear() {},
      select() {},
      destroy() {
        host.replaceChildren();
        inspector.replaceChildren();
      }
    });
  }

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  globalThis.CompassSemanticDiffGraph = Object.freeze({ mount });
})();
```

Use the existing SVG implementation as `renderCurrentGraph`; do not redesign
the layout in this task.

- [ ] **Step 4: Embed the JavaScript asset and mount the explorer**

After the embedded JSON and Pierre script, append a separate inline script:

```rust
output.push_str("<script>");
output.push_str(&SEMANTIC_DIFF_GRAPH_JS.replace("</script", "<\\/script"));
output.push_str("</script>");
```

Replace the old `renderChangedGraph(reportData.graph_delta)` call with:

```js
const graphExplorer = globalThis.CompassSemanticDiffGraph.mount({
  report: reportData,
  host: document.getElementById("graph-canvas"),
  inspector: document.getElementById("graph-inspector"),
  liveRegion: document.getElementById("graph-live"),
  note: document.getElementById("graph-note")
});
```

Change `render_graph_delta` to emit the approved explorer/inspector/live-region
markup. Preserve the current no-change empty state without mounting JavaScript.

- [ ] **Step 5: Add data hooks to exhaustive graph rows**

Render node rows with an escaped `data-graph-node-id`. Render edge rows with
escaped source, target, and relation attributes by using the existing
`html_attr` helper:

```rust
let _ = write!(
    output,
    "<li class=\"delta-row\" data-graph-node-id=\"{}\">",
    html_attr(&node.id)
);
```

Do not add report-derived content through raw HTML.

- [ ] **Step 6: Verify production compilation and formatting**

Run:

```bash
cargo fmt --all
cargo check -p compass-cli
```

Expected: both commands exit `0`; the report remains a single HTML document.

- [ ] **Step 7: Add renderer and CLI regression assertions**

Extend `html_report_is_standalone_exhaustive_and_escapes_report_data` and
`diff_emits_semantic_text_json_html_and_rejects_removed_flags` with:

```rust
assert!(html.contains("globalThis.CompassSemanticDiffGraph"));
assert!(html.contains("class=\"graph-explorer\""));
assert!(html.contains("id=\"graph-inspector\""));
assert!(html.contains("id=\"graph-live\""));
assert!(html.contains("data-graph-node-id=\"new\""));
assert!(html.contains("data-graph-edge-source=\"caller\""));
assert!(html.contains("Select a node to inspect its change."));
assert!(!html.contains("semantic-diff-graph.js"));
assert!(!html.contains("semantic-diff-graph.css"));
```

- [ ] **Step 8: Run focused Rust tests**

Run:

```bash
cargo test -p compass-cli \
  semantic_diff_render::tests::html_report_is_standalone_exhaustive_and_escapes_report_data \
  -- --exact
cargo test -p compass-cli --test history_cli \
  diff_emits_semantic_text_json_html_and_rejects_removed_flags \
  -- --exact
```

Expected: both tests pass.

- [ ] **Step 9: Commit the asset boundary**

```bash
git add \
  crates/compass-cli/assets/semantic-diff-graph.css \
  crates/compass-cli/assets/semantic-diff-graph.js \
  crates/compass-cli/src/semantic_diff_render.rs \
  crates/compass-cli/tests/history_cli.rs
git commit -m "refactor: isolate semantic diff graph explorer"
```

---

### Task 2: Implement readable capsules and deterministic collision-aware layout

**Files:**
- Modify: `crates/compass-cli/assets/semantic-diff-graph.js`
- Modify: `crates/compass-cli/assets/semantic-diff-graph.css`
- Modify: `tests/viewer/fixtures/generate.ts`
- Create: `tests/viewer/semantic-diff-graph.spec.ts`

**Interfaces:**
- Consumes: Task 1's `mount(options)` and stable graph DOM.
- Produces: `buildGraphModel(delta)`, `rankVisualNodes(model)`,
  `layoutVisualNodes(nodes, edges, width, height)`, and capsule SVG nodes with
  deterministic `data-node-id` attributes.

- [ ] **Step 1: Implement the normalized graph model**

Inside the JavaScript asset, define:

```js
const STATUS_PRIORITY = Object.freeze({
  context: 0,
  changed: 1,
  removed: 2,
  added: 3
});
const MAX_VISUAL_NODES = 42;
const MAX_VISUAL_EDGES = 100;

function buildGraphModel(delta) {
  const nodes = new Map();
  const edges = [
    ...(delta.changed_edges || []).map((edge) => ({ ...edge, status: "changed" })),
    ...(delta.removed_edges || []).map((edge) => ({ ...edge, status: "removed" })),
    ...(delta.added_edges || []).map((edge) => ({ ...edge, status: "added" }))
  ];
  // Remember node deltas first, then context-only edge endpoints.
  return { nodes, edges };
}
```

Every normalized node has:

```ts
type ExplorerNode = {
  id: string;
  label: string;
  kind: string;
  sourceFile: string;
  changedFields: string[];
  status: "added" | "removed" | "changed" | "context";
  degree: number;
};
```

When duplicate IDs occur, retain the higher-priority status and the richest
non-empty metadata. Count degree from all changed-edge records.

- [ ] **Step 2: Implement deterministic ranking and sampling**

Sort changed nodes by status priority, then degree descending, then identifier.
After direct deltas, include their connected context endpoints by degree and ID.
Finally include remaining context nodes by ID. Slice to
`MAX_VISUAL_NODES`. Filter edges to sampled endpoints, sort by
`source + relation + target + key`, and slice to `MAX_VISUAL_EDGES`.

The note must use:

```js
note.textContent =
  `Visual sample: ${visualNodes.length} of ${model.nodes.size} involved nodes `
  + `and ${visualEdges.length} of ${model.edges.length} changed edges. `
  + "The lists below and embedded JSON remain exhaustive.";
```

Only show this sampled note when either cap truncates data.

- [ ] **Step 3: Implement capsule sizing and layout**

Use deterministic dimensions:

```js
function capsuleWidth(node) {
  const visible = displayLabel(node).slice(0, 28);
  return Math.max(112, Math.min(210, 42 + visible.length * 6.4));
}

function displayLabel(node) {
  return node.label || node.id;
}
```

Initialize nodes on concentric rings from their ranked index. Run a fixed 180
iterations containing:

- rectangle-aware repulsion using half-width plus 12-pixel padding;
- an edge spring targeting 150 pixels;
- a weak center force;
- velocity damping; and
- bounds clamping using each capsule's half-width and 22-pixel half-height.

Never read wall-clock time or random values. The same report must produce the
same SVG coordinates.

- [ ] **Step 4: Render capsules instead of circles and free labels**

Each SVG group contains:

```html
<rect class="graph-node-surface" rx="8" ry="8"></rect>
<text class="graph-node-mark">+</text>
<text class="graph-node-label">RetryWithSmallerBatch</text>
<text class="graph-node-meta">function · batching.py</text>
```

Use `textContent` for every dynamic text node. Truncate the visible label to 28
characters and meta to 30; retain the complete accessible label on the group.
Render edges before nodes.

- [ ] **Step 5: Implement the capsule visual system**

The CSS asset must include:

```css
.graph-node {
  cursor: pointer;
  outline: none;
  transition: opacity 140ms ease;
}

.graph-node-surface {
  fill: var(--surface-raised);
  stroke: var(--muted);
  stroke-width: 1.3;
}

.graph-node.added .graph-node-surface { stroke: var(--green); }
.graph-node.removed .graph-node-surface {
  stroke: var(--red);
  stroke-dasharray: 4 3;
}
.graph-node.changed .graph-node-surface { stroke: var(--amber); }
.graph-node-label {
  fill: var(--text);
  font: 600 11px/1.2 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
.graph-node-meta {
  fill: var(--muted);
  font: 9px/1.2 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
```

Remove the old circle/free-label rules.

- [ ] **Step 6: Add the browser fixture using production assets**

In `tests/viewer/fixtures/generate.ts`, copy the two assets into
`fixtures/out/`, then generate `semanticDiffGraph.html` containing:

- a changed node `changed-core`;
- an added neighbor `added-leaf`;
- a removed incoming neighbor `removed-caller`;
- an unrelated node `unrelated`;
- a context-only endpoint `context-api`;
- 44 deterministic overflow context endpoints connected to `changed-core`, with
  `zz-outside-sample` sorting beyond the 42-node visual cap;
- one related semantic finding with ID `sd1-fixture`;
- one source change for `src/core.ts`; and
- the exact Task 1 explorer markup.

The fixture script reads its embedded JSON and calls the production
`CompassSemanticDiffGraph.mount`.

- [ ] **Step 7: Add layout and safety browser assertions**

Create `tests/viewer/semantic-diff-graph.spec.ts` with:

```ts
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/semanticDiffGraph.html");
});

test("renders deterministic readable node capsules", async ({ page }) => {
  const nodes = page.locator("#graph-canvas [data-node-id]");
  await expect(nodes).toHaveCount(42);
  await expect(page.getByText("changed-core", { exact: true })).toBeVisible();
  await expect(page.locator("#graph-canvas img")).toHaveCount(0);

  const boxes = await nodes.evaluateAll((elements) =>
    elements.map((element) => {
      const box = element.getBoundingClientRect();
      return { left: box.left, right: box.right, top: box.top, bottom: box.bottom };
    })
  );
  for (let left = 0; left < boxes.length; left += 1) {
    for (let right = left + 1; right < boxes.length; right += 1) {
      const overlaps = boxes[left].left < boxes[right].right
        && boxes[left].right > boxes[right].left
        && boxes[left].top < boxes[right].bottom
        && boxes[left].bottom > boxes[right].top;
      expect(overlaps).toBe(false);
    }
  }
});
```

Add a hostile label containing `</script><img src=x onerror=alert(1)>` and
assert it renders as text with no injected `img` element.

- [ ] **Step 8: Run focused Rust and Chromium checks**

Use the working Node installation explicitly:

```bash
cargo test -p compass-cli semantic_diff_render --lib
PATH=/Users/haipingfu/.nvm/versions/node/v24.13.1/bin:$PATH \
  npm exec --prefix tests/viewer playwright test semantic-diff-graph.spec.ts
```

Expected: Rust renderer tests and the Chromium capsule test pass.

- [ ] **Step 9: Commit the readable graph**

```bash
git add \
  crates/compass-cli/assets/semantic-diff-graph.css \
  crates/compass-cli/assets/semantic-diff-graph.js \
  tests/viewer/fixtures/generate.ts \
  tests/viewer/semantic-diff-graph.spec.ts
git commit -m "feat: render readable semantic diff graph"
```

---

### Task 3: Add neighborhood focus and the persistent inspector

**Files:**
- Modify: `crates/compass-cli/assets/semantic-diff-graph.js`
- Modify: `crates/compass-cli/assets/semantic-diff-graph.css`
- Modify: `tests/viewer/semantic-diff-graph.spec.ts`

**Interfaces:**
- Consumes: Task 2's normalized model and sampled SVG.
- Produces: `selectionFor(model, nodeId)`, `renderInspector(...)`,
  `select(nodeId)`, and `clear()` behavior returned by `mount`.

- [ ] **Step 1: Implement exhaustive relationship and finding indexes**

Build once during mount:

```js
function relationshipsFor(model, nodeId) {
  return {
    incoming: model.edges.filter((edge) => edge.target === nodeId),
    outgoing: model.edges.filter((edge) => edge.source === nodeId)
  };
}

function findingsFor(report, nodeId) {
  return (report.findings || []).filter((finding) =>
    finding.subject === nodeId
    || (finding.evidence || []).some((evidence) => evidence.record_key === nodeId)
  );
}
```

The model uses exhaustive edges, not the sampled visual-edge array.

- [ ] **Step 2: Implement selection and neighborhood classes**

`select(nodeId)` must:

1. resolve the normalized node, including out-of-sample context nodes;
2. compute direct neighbor IDs from exhaustive edges;
3. set selected SVG node `aria-pressed="true"` and `.is-selected`;
4. apply `.is-neighbor` to direct visible neighbors;
5. apply `.is-dimmed` to unrelated visible nodes;
6. apply `.is-related` to touching visible edges and `.is-dimmed` elsewhere;
7. render the inspector; and
8. announce `Inspecting {label}` in the live region.

`clear()` removes all selection classes, resets `aria-pressed`, renders the
empty inspector, and announces `Graph selection cleared`.

- [ ] **Step 3: Render the inspector with text-only DOM APIs**

Render this semantic hierarchy:

```html
<header class="graph-inspector-header">
  <span class="graph-status added">Added</span>
  <h3 id="graph-inspector-heading">RetryWithSmallerBatch</h3>
  <code>python_..._retrywithsmallerbatch</code>
</header>
<dl class="graph-inspector-facts">
  <div><dt>Kind</dt><dd>function</dd></div>
  <div><dt>Source</dt><dd>python/.../batching.py</dd></div>
  <div><dt>Changed fields</dt><dd>signature, implementation</dd></div>
</dl>
<section>
  <h4>Incoming relationships</h4>
  <button data-neighbor-id="caller">caller <span>calls · added</span></button>
</section>
<section>
  <h4>Outgoing relationships</h4>
  ...
</section>
<section>
  <h4>Related findings</h4>
  <a href="#sd1-fixture">Behavior changed</a>
</section>
```

For unknown kind/source, omit the fact row. For empty relationship/finding
groups, render `No changed incoming relationships`, `No changed outgoing
relationships`, or `No related semantic findings`.

- [ ] **Step 4: Connect pointer selection and inspector neighbor navigation**

Use one delegated click listener on the host and one on the inspector. Find the
nearest `[data-node-id]` or `[data-neighbor-id]`, read its dataset, and call
`select`. A click whose target is the SVG background calls `clear`.

Do not interpolate node IDs into selector strings; use maps and dataset values.

- [ ] **Step 5: Style the change lens and inspector**

Add:

```css
.graph-node.is-selected .graph-node-surface {
  stroke: var(--accent);
  stroke-width: 2.5;
}
.graph-node.is-neighbor { opacity: 1; }
.graph-node.is-dimmed,
.graph-edge.is-dimmed { opacity: .18; }
.graph-edge.is-related {
  opacity: 1;
  stroke-width: 2.2;
}
.graph-inspector-header {
  padding-bottom: 14px;
  border-bottom: 1px solid var(--border);
}
.graph-inspector-section {
  padding-top: 14px;
  border-top: 1px solid var(--border);
}
.graph-relation {
  display: block;
  width: 100%;
  min-height: 0;
  padding: 8px 0;
  border: 0;
  border-bottom: 1px solid var(--border);
  border-radius: 0;
  background: transparent;
  text-align: left;
}
```

Preserve the original added/removed/changed border when a connected neighbor is
focused. Selection blue is an outer emphasis, not a replacement for status.

- [ ] **Step 6: Add browser interaction assertions**

Add:

```ts
test("selects a node, focuses its neighborhood, and navigates relationships", async ({ page }) => {
  const changed = page.locator('[data-node-id="changed-core"]');
  await changed.click();

  await expect(changed).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('[data-node-id="added-leaf"]')).toHaveClass(/is-neighbor/);
  await expect(page.locator('[data-node-id="unrelated"]')).toHaveClass(/is-dimmed/);
  await expect(page.getByRole("heading", { name: "changed-core" })).toBeVisible();

  await page.locator('[data-neighbor-id="context-api"]').click();
  await expect(page.locator("#graph-live")).toContainText("Inspecting context-api");

  await page.locator("#graph-canvas svg").click({ position: { x: 4, y: 4 } });
  await expect(page.locator('[data-node-id="changed-core"]'))
    .toHaveAttribute("aria-pressed", "false");
});
```

Also assert the inspector reports the expected incoming and outgoing relation
labels and links `sd1-fixture`.

- [ ] **Step 7: Run focused browser and Rust regression checks**

```bash
PATH=/Users/haipingfu/.nvm/versions/node/v24.13.1/bin:$PATH \
  npm exec --prefix tests/viewer playwright test semantic-diff-graph.spec.ts
cargo test -p compass-cli semantic_diff_render --lib
```

Expected: selection, inspector, and renderer tests pass.

- [ ] **Step 8: Commit selection and inspection**

```bash
git add \
  crates/compass-cli/assets/semantic-diff-graph.css \
  crates/compass-cli/assets/semantic-diff-graph.js \
  tests/viewer/semantic-diff-graph.spec.ts
git commit -m "feat: inspect changed graph nodes"
```

---

### Task 4: Complete navigation, keyboard, responsive, and failure behavior

**Files:**
- Modify: `crates/compass-cli/src/semantic_diff_render.rs`
- Modify: `crates/compass-cli/assets/semantic-diff-graph.js`
- Modify: `crates/compass-cli/assets/semantic-diff-graph.css`
- Modify: `crates/compass-cli/tests/history_cli.rs`
- Modify: `tests/viewer/semantic-diff-graph.spec.ts`
- Modify: `docs/reference/outputs.md:264-279`
- Modify: `docs/guides/versioned-history.md:239-272`

**Interfaces:**
- Consumes: Tasks 1–3 explorer and inspector.
- Produces: valid source/finding navigation, out-of-sample selection state,
  keyboard equivalence, responsive inspector, and safe runtime fallback.

- [ ] **Step 1: Add stable source-change anchors**

In `render_source_changes`, add the source index to each details element:

```rust
let _ = write!(
    output,
    "<details id=\"source-change-{index}\" class=\"source-file\" \
     data-source-index=\"{index}\"{}>",
    if index == 0 { " open" } else { "" }
);
```

Finding cards already use their validated `sd1-...` IDs. Do not create links
for arbitrary finding IDs that are absent from the DOM.

- [ ] **Step 2: Implement valid navigation targets**

Build a source target map by comparing normalized slash-separated
`node.sourceFile` with each source change's `old_path` and `new_path`.

Inspector links are:

```js
sourceLink.href = `#source-change-${sourceIndex}`;
findingLink.href = `#${finding.id}`;
```

Create them only after verifying the corresponding DOM element exists. Add a
`Show in exhaustive list` button only when a row with matching
`data-graph-node-id` exists; on activation, call `scrollIntoView` and focus the
row temporarily with `tabindex="-1"`.

- [ ] **Step 3: Implement keyboard behavior**

On the graph host:

```js
host.addEventListener("keydown", (event) => {
  const node = event.target.closest("[data-node-id]");
  if (node && (event.key === "Enter" || event.key === " ")) {
    event.preventDefault();
    select(node.dataset.nodeId);
  }
  if (event.key === "Escape") {
    event.preventDefault();
    clear();
  }
});
```

Inspector relationship buttons already use native keyboard semantics. Clicking
or activating an out-of-sample neighbor updates the inspector and live region;
the note becomes:

```text
Inspecting a node outside the bounded visual sample. Its changed relationships
remain available here and in the exhaustive lists.
```

- [ ] **Step 4: Implement runtime fallback and cleanup**

Wrap mount initialization in `try/catch`. On failure:

```js
host.replaceChildren(
  element("p", "graph-render-fallback",
    "Interactive graph unavailable. Use the exhaustive node and edge lists below.")
);
note.textContent =
  "The embedded report data and exhaustive graph-change lists remain available.";
console.warn("Compass could not render the changed graph", error);
```

`destroy()` removes host and inspector event listeners before clearing DOM.

- [ ] **Step 5: Finish responsive, focus, and reduced-motion CSS**

Add:

```css
.graph-node:focus-visible .graph-node-surface,
.graph-relation:focus-visible,
.graph-inspector a:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

@media (max-width: 760px) {
  .graph-explorer { grid-template-columns: 1fr; }
  .graph-canvas {
    min-height: 390px;
    border-right: 0;
    border-bottom: 1px solid var(--border);
  }
  .graph-inspector { min-height: 220px; }
}

@media (prefers-reduced-motion: reduce) {
  .graph-node,
  .graph-edge { transition: none; }
}

@media (forced-colors: active) {
  .graph-node-surface { stroke: CanvasText; }
  .graph-node.is-selected .graph-node-surface { stroke: Highlight; }
  .graph-edge.is-related { stroke: Highlight; }
}
```

Use SVG stroke styling rather than CSS `outline` on SVG groups if Chromium does
not paint the group outline; retain the same visible focus requirement.

- [ ] **Step 6: Extend Rust and CLI output assertions**

Assert:

```rust
assert!(html.contains("id=\"source-change-0\""));
assert!(html.contains("aria-live=\"polite\""));
assert!(html.contains("Interactive graph unavailable."));
assert!(html.contains("@media (max-width: 760px)"));
assert!(html.contains("@media (prefers-reduced-motion: reduce)"));
assert!(!html.contains("href=\"#source-change-undefined\""));
```

- [ ] **Step 7: Add keyboard, responsive, and fallback browser coverage**

Add tests that:

- focus `changed-core`, press Enter, and see `aria-pressed="true"`;
- press Escape and see selection cleared;
- select the context-only endpoint and see unavailable kind/source omitted;
- select the out-of-sample fixture neighbor and see the bounded-sample note;
- set viewport width to 720 and assert inspector top is at or below canvas
  bottom;
- assert the source link targets `#source-change-0`;
- assert the finding link targets `#sd1-fixture`; and
- intentionally remove `report.graph_delta` before mount and verify the safe
  fallback without removing exhaustive list fixture content.

- [ ] **Step 8: Document how reviewers use the explorer**

In `docs/reference/outputs.md`, state that the graph:

- is a bounded visual sample backed by exhaustive lists and JSON;
- uses selection to focus direct changed-edge neighborhoods;
- shows only retained node/edge metadata;
- links to findings and source patches when exact targets exist; and
- remains useful without JavaScript through exhaustive lists.

In `docs/guides/versioned-history.md`, add this reading order:

```text
Select a changed node, inspect incoming and outgoing changed relationships,
follow related semantic findings, then open the exact source patch. Clear the
selection to restore the whole sampled topology.
```

- [ ] **Step 9: Run focused checks**

```bash
cargo fmt --all -- --check
cargo test -p compass-cli semantic_diff_render --lib
cargo test -p compass-cli --test history_cli \
  diff_emits_semantic_text_json_html_and_rejects_removed_flags \
  -- --exact
PATH=/Users/haipingfu/.nvm/versions/node/v24.13.1/bin:$PATH \
  npm exec --prefix tests/viewer playwright test semantic-diff-graph.spec.ts
git diff --check
```

Expected: all commands pass with no whitespace errors.

- [ ] **Step 10: Commit the complete interaction**

```bash
git add \
  crates/compass-cli/src/semantic_diff_render.rs \
  crates/compass-cli/assets/semantic-diff-graph.css \
  crates/compass-cli/assets/semantic-diff-graph.js \
  crates/compass-cli/tests/history_cli.rs \
  tests/viewer/semantic-diff-graph.spec.ts \
  docs/reference/outputs.md \
  docs/guides/versioned-history.md
git commit -m "feat: complete semantic diff graph interaction"
```

---

### Task 5: Qualify the completed report on CocoIndex and run release gates

**Files:**
- Modify only if qualification reveals a defect in the files from Tasks 1–4.
- Regenerate, do not commit: `tests/viewer/fixtures/out/*`
- Generate outside the repository:
  `/tmp/compass-cocoindex-interactive-semantic-diff.html`

**Interfaces:**
- Consumes: the complete standalone graph explorer.
- Produces: fresh real-repository evidence, deterministic output evidence, and
  final verification results.

- [ ] **Step 1: Refresh the Graphify knowledge graph**

```bash
graphify update .
```

Expected: update completes without API cost and reports refreshed Compass graph
counts.

- [ ] **Step 2: Run formatting, strict focused lint, and Rust tests**

```bash
cargo fmt --all -- --check
cargo clippy -p compass-cli --all-targets --no-deps -- -D warnings
cargo test -p compass-cli semantic_diff_render --lib
cargo test -p compass-cli --test history_cli \
  diff_emits_semantic_text_json_html_and_rejects_removed_flags \
  -- --exact
cargo test -p compass-cli --test history_cli \
  semantic_diff_end_to_end_languages \
  -- --exact --nocapture
```

Expected: all commands exit `0`.

- [ ] **Step 3: Run the browser suite for the production assets**

```bash
PATH=/Users/haipingfu/.nvm/versions/node/v24.13.1/bin:$PATH \
  npm exec --prefix tests/viewer playwright test semantic-diff-graph.spec.ts
```

Expected: all graph explorer Chromium tests pass.

- [ ] **Step 4: Rebuild Compass from the current commit**

```bash
cargo build -p compass-cli
./target/debug/compass --version
```

Expected: build exits `0` and the command reports the current Compass version.

- [ ] **Step 5: Generate a fresh real CocoIndex report**

Run from `/Volumes/workspace/Github/cocoindex-compass-audit-20260726`:

```bash
/Users/haipingfu/graphify/compass/target/debug/compass diff \
  90571539fa291fc6e6b248095bd2c8a2ff68bab4 \
  71f9cc9dc693080310181a2d011fb737420f7907 \
  --format html \
  --output /tmp/compass-cocoindex-interactive-semantic-diff.html \
  </dev/null
```

Expected: the command exits `0` and writes one self-contained report.

- [ ] **Step 6: Verify the real report's interaction contract and evidence**

```bash
REAL_REPORT=/tmp/compass-cocoindex-interactive-semantic-diff.html
test -s "$REAL_REPORT"
rg -q 'globalThis.CompassSemanticDiffGraph' "$REAL_REPORT"
rg -q 'class="graph-explorer"' "$REAL_REPORT"
rg -q 'id="graph-inspector"' "$REAL_REPORT"
rg -q 'data-graph-node-id=' "$REAL_REPORT"
rg -q 'if not _is_global_litellm_error\\(e\\)' "$REAL_REPORT"
rg -q 'sd1-6a303dab4ee88cb6df047cce' "$REAL_REPORT"
if rg -q '<script src=|<link rel=' "$REAL_REPORT"; then exit 1; fi
```

Expected: every required contract/evidence check passes and no external asset
tag exists.

- [ ] **Step 7: Verify deterministic JSON and a clean upstream checkout**

Run from the CocoIndex audit repository:

```bash
COMPASS_BIN=/Users/haipingfu/graphify/compass/target/debug/compass
OLD_REV=90571539fa291fc6e6b248095bd2c8a2ff68bab4
NEW_REV=71f9cc9dc693080310181a2d011fb737420f7907
HASH_ONE=$("$COMPASS_BIN" diff "$OLD_REV" "$NEW_REV" --format json \
  | shasum -a 256 | awk '{print $1}')
HASH_TWO=$("$COMPASS_BIN" diff "$OLD_REV" "$NEW_REV" --format json \
  | shasum -a 256 | awk '{print $1}')
test "$HASH_ONE" = "$HASH_TWO"
test -z "$(git status --porcelain=v1)"
printf 'semantic_diff_sha256=%s\ncheckout_clean=yes\n' "$HASH_ONE"
```

Expected: hashes match and the upstream checkout remains clean.

- [ ] **Step 8: Run final workspace and whitespace gates**

```bash
cargo test --workspace
git diff --check
git status -sb
```

Expected: workspace tests pass; only known user-owned untracked directories
remain; no implementation changes are left uncommitted.

- [ ] **Step 9: If qualification required fixes, commit them**

Only when Steps 1–8 exposed and you fixed a defect:

```bash
git add \
  crates/compass-cli/src/semantic_diff_render.rs \
  crates/compass-cli/assets/semantic-diff-graph.css \
  crates/compass-cli/assets/semantic-diff-graph.js \
  crates/compass-cli/tests/history_cli.rs \
  tests/viewer/fixtures/generate.ts \
  tests/viewer/semantic-diff-graph.spec.ts \
  docs/reference/outputs.md \
  docs/guides/versioned-history.md
git commit -m "fix: qualify semantic diff graph explorer"
```

Do not create an empty commit when qualification required no fixes.
