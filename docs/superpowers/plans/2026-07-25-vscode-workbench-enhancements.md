# VS Code Workbench Enhancements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a polished Compass graph-loading experience, a collapsible and
resizable inspector, quiet automatic CLI discovery, actionable Repository and
Operations trees, and discoverable Git revision graphs.

**Architecture:** Keep graph export, validation, and command execution in the VS
Code host. Put reusable inspector layout behavior in `@compass/viewer`, keep
loading/error presentation in the graph webview, and derive native VS Code trees
from pure descriptor builders.

**Tech Stack:** TypeScript 5.9, React 19, VS Code Extension API 1.95, Zod 4,
Vitest 3, Testing Library, Vite 7, esbuild 0.25, and Playwright.

## Execution method

This is an implementation-first plan, not a test-driven-development plan.
Implement each task's behavior first, then add or update tests to verify the
completed behavior. A task is complete only when its expected outcome and
verification commands both succeed.

## Global constraints

- Preserve local-only processing and existing Compass CLI capability contracts.
- Do not bundle or download the Compass CLI.
- Resolve a configured executable first, then fall back to `compass` on `PATH`.
- Do not display a healthy CLI path as a permanent Repository row.
- Store repository graph artifacts only under `<repository>/compass-out/`.
- Do not create or use a `graphify-out/` directory for this feature.
- Do not build historical graphs implicitly.
- Use existing extension commands as the execution path for tree actions.
- Preserve VS Code high-contrast themes and `prefers-reduced-motion`.
- Preserve the stacked inspector layout below `760px`.
- Preserve unrelated user changes and untracked files.

---

## Task 1: Flexible graph inspector

**Context:** `CompassGraph` currently uses a fixed `340px` inspector column.
`GraphInspector` has no collapse control, and the canvas cannot reclaim inspector
space. The inspector is part of the shared viewer, so this capability belongs in
`@compass/viewer`, not in a VS Code-only wrapper.

**Task goal:** Add a right-docked inspector that users can resize with a pointer or
keyboard, collapse into a narrow rail, and expand again.

**Expected outcome:**

- Default offline and VS Code graphs still open with a `340px` inspector.
- Width is clamped from `280px` through `560px`.
- Dragging the separator changes the right inspector width.
- Left/Right arrows resize it in `24px` increments.
- Collapse leaves a `48px` right rail with an expand control.
- Narrow layouts retain the stacked inspector experience.
- Existing search, inspection, communities, and source navigation still work.

**Files:**

- Create: `packages/compass-viewer/src/graph/inspectorLayout.ts`
- Create: `packages/compass-viewer/src/graph/InspectorResizeHandle.tsx`
- Create: `packages/compass-viewer/src/graph/inspectorLayout.test.ts`
- Create: `packages/compass-viewer/src/graph/InspectorResizeHandle.test.tsx`
- Modify: `packages/compass-viewer/src/graph/CompassGraph.tsx`
- Modify: `packages/compass-viewer/src/graph/GraphInspector.tsx`
- Modify: `packages/compass-viewer/src/theme.css`
- Modify: `packages/compass-viewer/src/index.ts`
- Modify: `tests/viewer/accessibility.spec.ts`

- [ ] **Step 1.1: Implement the layout model**

**Step context:** Width math and stored-state normalization should be independent
of React and browser events.

**Step goal:** Provide one reusable source of truth for inspector dimensions.

**Action:** Add:

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

export function clampInspectorWidth(width: number): number;
export function normalizeInspectorLayout(
  value: Partial<InspectorLayout> | undefined
): InspectorLayout;
export function resizeInspectorFromPointer(
  containerRight: number,
  clientX: number
): number;
export function resizeInspectorByKeyboard(width: number, key: string): number;
```

Export the layout type and functions from `packages/compass-viewer/src/index.ts`.

**Expected:** Invalid or stale stored widths cannot break the graph layout; every
consumer receives a normalized `InspectorLayout`.

- [ ] **Step 1.2: Implement the accessible resize handle**

**Step context:** The separator sits between the graph stage and the right-docked
inspector.

**Step goal:** Support equivalent pointer and keyboard resizing.

**Action:** Create `InspectorResizeHandle` with:

```tsx
export function InspectorResizeHandle({
  width,
  onResize
}: {
  width: number;
  onResize(width: number): void;
});
```

Use `role="separator"`, `aria-orientation="vertical"`, current/minimum/maximum
ARIA values, pointer capture during dragging, and Left/Right key handling.

**Expected:** The separator is focusable, visibly focused, screen-reader
identifiable, and does not continue resizing after pointer release.

- [ ] **Step 1.3: Integrate layout state with `CompassGraph`**

**Step context:** Offline exports need defaults, while VS Code needs to supply and
persist a layout.

**Step goal:** Extend the shared viewer without breaking existing callers.

**Action:** Change the graph props to:

```ts
export type CompassGraphProps = {
  model: GraphViewModel;
  host: GraphHost;
  initialInspectorLayout?: Partial<InspectorLayout>;
  onInspectorLayoutChange?(layout: InspectorLayout): void;
};
```

Normalize initial state once. Render the separator only while expanded. Pass
`collapsed` and `onToggleCollapsed` into `GraphInspector`. Publish every committed
width/collapse change through `onInspectorLayoutChange`.

**Expected:** Existing callers compile unchanged, while hosts that provide layout
state receive deterministic change notifications.

- [ ] **Step 1.4: Add collapse/expand controls and responsive styling**

**Step context:** The right inspector already owns its header and content, making
it the correct place for disclosure controls.

**Step goal:** Reclaim graph space without losing inspector discoverability.

**Action:** Use `PanelRightCloseIcon` and `PanelRightOpenIcon`. Add
`Collapse graph inspector` to the expanded header and `Expand graph inspector` to
the collapsed rail. Drive the desktop grid with
`--compass-inspector-width`. Hide the drag handle in the existing `760px` media
query. Add focus and high-contrast styling and keep nonessential transitions under
the reduced-motion rule.

**Expected:** Expanded, collapsed, high-contrast, reduced-motion, and narrow-screen
states remain readable and operable.

- [ ] **Step 1.5: Add verification coverage after implementation**

**Step context:** The completed layout behavior now has stable interfaces to test.

**Step goal:** Protect normalization, keyboard behavior, and accessibility.

**Action:** Add unit cases for clamping, stored state, pointer width, and keyboard
width. Add Testing Library coverage for separator ARIA attributes. Extend the
Playwright accessibility spec to collapse and expand the inspector.

**Expected:** The tests prove behavior rather than implementation details.

- [ ] **Step 1.6: Verify and commit**

Run:

```bash
npm test -w @compass/viewer
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
node scripts/build_viewer_assets.mjs
npx playwright test tests/viewer/accessibility.spec.ts tests/viewer/graph-parity.spec.ts
```

**Expected:** All commands pass and embedded viewer assets match the new source.

Commit:

```bash
git add packages/compass-viewer/src/graph packages/compass-viewer/src/theme.css packages/compass-viewer/src/index.ts tests/viewer/accessibility.spec.ts crates/compass-output/assets/viewer
git commit -m "feat(viewer): make graph inspector flexible"
```

---

## Task 2: Polished loading and recoverable graph errors

**Context:** `GraphPanel.html()` currently displays a small unstyled sentence in
the top-left corner. Hydration errors replace it with a basic message, and recovery
requires reopening the graph.

**Task goal:** Render a centered Compass constellation immediately, transition to
the hydrated graph, and provide Retry and Show Compass output after failure.

**Expected outcome:**

- Opening a graph immediately shows `Mapping your codebase`.
- Decorative nodes and edges are hidden from assistive technology.
- Reduced-motion users do not receive ambient pulses.
- Hydration success replaces the loader with `CompassGraph`.
- Failure provides the real host error plus Retry and Show Compass output.
- Retry reruns hydration in the same tab.
- Inspector state survives graph tab visibility changes and re-renders.

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

- [ ] **Step 2.1: Implement the loading/error component**

**Step context:** The webview should own visual state while the extension host owns
CLI execution.

**Step goal:** Give loading and failure a consistent, useful presentation.

**Action:** Add:

```ts
export type GraphLoadState =
  | { kind: "loading" }
  | { kind: "error"; message: string };
```

`GraphLoadingState` receives `state`, `onRetry`, and `onShowOutput`. Use the approved
copy:

- `Compass graph`
- `Mapping your codebase`
- `Reading graph · Arranging relationships · Preparing inspector`
- `Compass could not load this graph`

Build the constellation with CSS and inline SVG icons; do not add image files.

**Expected:** The screen is centered and theme-aware in light, dark, and
high-contrast VS Code themes.

- [ ] **Step 2.2: Add typed host recovery messages**

**Step context:** All graph webview messages are schema-validated before the host
acts.

**Step goal:** Keep retry and output actions inside the existing trusted message
boundary.

**Action:** Add `{ type: "retry" }` and `{ type: "showOutput" }` to
`GraphToHostMessageSchema`. Pass the Compass `OutputChannel` into
`GraphPanel.open()`. Treat `ready` and `retry` as hydration requests; call
`output.show(true)` for `showOutput`.

**Expected:** Unknown messages remain ignored, while the two new actions are
validated and deterministic.

- [ ] **Step 2.3: Render webview states and persist inspector layout**

**Step context:** VS Code provides `getState()` and `setState()` specifically for
webview-local persistence.

**Step goal:** Make loading immediate and inspector preferences stable within the
tab.

**Action:** Extend the local API declaration:

```ts
type WebviewState = { inspector?: InspectorLayout };

declare function acquireVsCodeApi(): {
  postMessage(message: unknown): void;
  getState(): WebviewState | undefined;
  setState(state: WebviewState): void;
};
```

Render loading before posting `ready`. On hydration, pass stored layout to
`CompassGraph` and persist changes. On retry, render loading before posting the
retry request. Remove the raw loading text from the host HTML.

**Expected:** Users never see the old top-left sentence or an empty graph tab.

- [ ] **Step 2.4: Add verification coverage after implementation**

**Step context:** The component and message contracts now exist.

**Step goal:** Verify visible copy, ARIA roles, action callbacks, and schema
acceptance.

**Action:** Add Testing Library dependencies to the VS Code workspace as dev-only
packages. Test loading status, decorative `aria-hidden`, error alert, Retry, and
Show Compass output. Extend schema tests or add focused assertions for both new
messages.

**Expected:** Tests fail if the loading copy, recovery actions, or trusted message
contracts regress.

- [ ] **Step 2.5: Verify and commit**

Run:

```bash
npm test -w editors/vscode -- GraphLoadingState.test.tsx
npm run typecheck -w editors/vscode
npm run build -w @compass/viewer
node scripts/build_viewer_assets.mjs
npm run build -w editors/vscode
```

**Expected:** Component tests, type checking, shared CSS generation, and the
production extension bundle pass.

Commit:

```bash
git add editors/vscode/src/webviews editors/vscode/src/transport/messages.ts editors/vscode/src/views/graphPanel.ts editors/vscode/src/extension.ts editors/vscode/package.json package-lock.json packages/compass-viewer/src/theme.css crates/compass-output/assets/viewer
git commit -m "feat(vscode): add polished graph loading state"
```

---

## Task 3: Automatic CLI discovery and useful Repository tree

**Context:** Discovery already checks `PATH`, but an invalid configured path
currently prevents fallback. The Repository tree permanently shows the full CLI
path even when it is healthy, while repository actions are not exposed as child
items.

**Task goal:** Make CLI discovery resilient and make Repository describe the
workspace rather than internal setup.

**Expected outcome:**

- A valid configured binary wins.
- An invalid configured binary falls back to `PATH`.
- A healthy CLI path is not shown in Repository.
- Missing or incompatible CLI state produces one actionable setup row.
- Each repository row displays graph state and contextual child actions.
- Available graphs expose Open graph and Codebase evolution.
- Missing graphs expose Initialize repository.
- Failed graphs expose Update graph.

**Files:**

- Create: `editors/vscode/src/views/treeModel.ts`
- Create: `editors/vscode/src/views/treeModel.test.ts`
- Modify: `editors/vscode/src/cli/discovery.ts`
- Modify: `editors/vscode/src/cli/discovery.test.ts`
- Modify: `editors/vscode/src/views/statusTree.ts`
- Modify: `editors/vscode/src/extension.ts`

- [ ] **Step 3.1: Make discovery fall back after configured-path failure**

**Step context:** Remote SSH, WSL, and Dev Containers use the extension host's
environment, so `PATH` remains the reliable automatic fallback.

**Step goal:** Avoid forcing manual selection when a working CLI is already
available.

**Action:** Build a deduplicated candidate list in this order:

```ts
const candidates = [
  ...(configured ? [configured] : []),
  ...pathCandidates
].filter((candidate, index, all) => all.indexOf(candidate) === index);
```

Keep the existing executable checks and Windows filename variants.

**Expected:** Discovery returns the first executable candidate and reports every
searched candidate only when all checks fail.

- [ ] **Step 3.2: Implement pure tree descriptors**

**Step context:** Unit tests should not need to import the `vscode` module.

**Step goal:** Separate tree decisions from native TreeItem rendering.

**Action:** Define:

```ts
export type TreeNode = {
  id: string;
  label: string;
  description?: string;
  tooltip?: string;
  icon: string;
  command?: string;
  commandArguments?: unknown[];
  children?: TreeNode[];
};
```

Add `buildRepositoryTree(discovery, sessions)`. Use `path.basename(root)` for the
repository label and preserve the full root in the tooltip.

**Expected:** Repository content can be fully tested with plain objects.

- [ ] **Step 3.3: Render the nested native Repository tree**

**Step context:** `StatusTree` currently returns a flat `TreeItem[]`.

**Step goal:** Preserve native VS Code behavior while exposing contextual actions.

**Action:** Make `StatusTree` a `TreeDataProvider<TreeNode>`. Convert descriptors
to `TreeItem` in `getTreeItem()`, including icons, tooltips, commands, arguments,
and collapsible state. Return descriptor children from `getChildren(element)`.
Pass the full `CompassDiscovery` result from activation.

**Expected:** Repository rows expand natively and action items execute the existing
commands.

- [ ] **Step 3.4: Add verification coverage after implementation**

**Step context:** Discovery and tree decisions now have pure inputs and outputs.

**Step goal:** Cover success, fallback, missing, incompatible, available,
not-materialized, and failed states.

**Action:** Add unit tests for configured precedence, configured failure with PATH
success, hidden healthy CLI, setup rows, and repository child commands.

**Expected:** A regression cannot reintroduce the permanent healthy CLI path row or
remove the history action.

- [ ] **Step 3.5: Verify and commit**

Run:

```bash
npm test -w editors/vscode -- discovery.test.ts treeModel.test.ts
npm run typecheck -w editors/vscode
```

**Expected:** Discovery and Repository tests pass without a VS Code integration
host.

Commit:

```bash
git add editors/vscode/src/cli editors/vscode/src/views/treeModel.ts editors/vscode/src/views/treeModel.test.ts editors/vscode/src/views/statusTree.ts editors/vscode/src/extension.ts
git commit -m "feat(vscode): streamline repository setup"
```

---

## Task 4: Complete Operations command center

**Context:** Operations currently shows only Building, Watching, or No active
operations. Users cannot discover or launch the extension's existing workflows
from that view. Session refresh also risks replacing transient building/failed
states with a filesystem-only state.

**Task goal:** Turn Operations into a grouped command center while keeping active
work visible and accurate.

**Expected outcome:**

- Active builds and watches appear first.
- Build contains Initialize or Update plus Start/Stop watch as appropriate.
- Explore contains Open graph, Call graph from cursor, Architecture flow, and Query
  codebase when a graph is available.
- History contains Codebase evolution.
- Items invoke existing command handlers.
- Multi-root commands still use the existing repository picker.
- Building remains visible while a writer is active.
- Failed state remains visible until a later successful materialization.

**Files:**

- Modify: `editors/vscode/src/views/treeModel.ts`
- Modify: `editors/vscode/src/views/treeModel.test.ts`
- Modify: `editors/vscode/src/views/operationsTree.ts`
- Modify: `editors/vscode/src/workspace/sessionRegistry.ts`
- Create: `editors/vscode/src/workspace/sessionRegistry.test.ts`
- Modify: `editors/vscode/src/commands/buildCommands.ts`

- [ ] **Step 4.1: Implement grouped Operations descriptors**

**Step context:** The extension already registers every requested action.

**Step goal:** Improve discovery without duplicating command execution logic.

**Action:** Add `buildOperationsTree(sessions)` with groups in this order:

1. Active operations, only when non-empty.
2. Build.
3. Explore, when at least one graph is available.
4. History.

Use these command IDs:

```ts
"compass.initialize"
"compass.update"
"compass.toggleWatch"
"compass.openGraph"
"compass.openCallGraph"
"compass.openArchitecture"
"compass.openQuery"
"compass.openHistory"
```

**Expected:** Operations always offers meaningful actions and never falls back to
the passive `No active operations` row.

- [ ] **Step 4.2: Render grouped Operations as a native tree**

**Step context:** Groups and actions need native expansion, icons, and command
dispatch.

**Step goal:** Keep the activity-bar experience consistent with VS Code.

**Action:** Convert `OperationsTree` to `TreeDataProvider<TreeNode>`. Expand Active
operations by default, collapse command groups by default, and map action commands
through the same descriptor renderer used by Repository where practical.

**Expected:** All actions are keyboard accessible and use VS Code theme icons.

- [ ] **Step 4.3: Preserve transient operation state**

**Step context:** `SessionRegistry.refresh()` currently derives state from graph
file existence alone.

**Step goal:** Keep UI state aligned with in-process work.

**Action:** Add:

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

Assign `session.activeWriter` before refreshing at operation start. Clear it only
when the same operation completes.

**Expected:** Repository and Operations show building immediately and do not erase
failed state during the final refresh.

- [ ] **Step 4.4: Add verification coverage after implementation**

**Step context:** Operations grouping and session-state resolution are now pure.

**Step goal:** Verify action availability and transient states.

**Action:** Test missing/available graphs, active build/watch ordering, watch label
changes, command IDs, building precedence, materialized success, and preserved
failure.

**Expected:** Tests describe the complete Operations surface and operation-state
lifecycle.

- [ ] **Step 4.5: Verify and commit**

Run:

```bash
npm test -w editors/vscode -- treeModel.test.ts sessionRegistry.test.ts
npm run typecheck -w editors/vscode
```

**Expected:** Operations and session-state verification pass.

Commit:

```bash
git add editors/vscode/src/views editors/vscode/src/workspace/sessionRegistry.ts editors/vscode/src/workspace/sessionRegistry.test.ts editors/vscode/src/commands/buildCommands.ts
git commit -m "feat(vscode): complete operations command center"
```

---

## Task 5: Discoverable Git and build history

**Context:** The Codebase Evolution workspace already lists reachable Git commits,
loads available revision graphs, explicitly builds missing ones, compares parents,
and queries revisions. Its command exists but is difficult to discover.

**Task goal:** Surface the existing history workflow in Repository, Operations, and
the Repository view title, then explain how both activity-bar sections work.

**Expected outcome:**

- Codebase evolution is visible under Repository and Operations.
- A history icon appears in the Repository view title.
- Users understand the difference between Git commits and materialized graph
  builds.
- Documentation states that history never builds revisions implicitly.
- Existing history safety and profile selection remain unchanged.

**Files:**

- Modify: `editors/vscode/package.json`
- Modify: `editors/vscode/README.md`
- Modify: `editors/vscode/src/test/suite/extension.integration.ts`

- [ ] **Step 5.1: Add the Repository title history action**

**Step context:** Native view-title actions remain visible even when tree groups are
collapsed.

**Step goal:** Provide a one-click entry to Git/build history.

**Action:** Add:

```json
{
  "command": "compass.openHistory",
  "when": "view == compass.status",
  "group": "navigation@2"
}
```

**Expected:** Repository offers Open Graph, Codebase Evolution, and Update in its
native title area.

- [ ] **Step 5.2: Document Repository, Operations, and Codebase Evolution**

**Step context:** The user requested an explanation of these surfaces, not only
implementation.

**Step goal:** Make the workflows understandable without reading source code.

**Action:** Add `Using the Compass activity bar` to the extension README:

- Repository describes workspace/graph health and contextual actions.
- Operations launches Build, Explore, and History workflows and shows active work.
- Codebase Evolution lists reachable commits and graph materialization state.
- Select a commit, choose Build graph when missing, then Open graph, Compare
  parent, or Query this revision.
- Opening history never creates a missing revision graph.

**Expected:** The README answers what both panels do and how to inspect historical
graphs.

- [ ] **Step 5.3: Add integration verification after implementation**

**Step context:** Command contribution and registration are extension-host
behaviors.

**Step goal:** Ensure the history entry cannot disappear from the UI manifest.

**Action:** Extend the integration test to assert all primary commands remain
registered and `compass.openHistory` is contributed to `view == compass.status`.

**Expected:** The test fails if either history command registration or the title
menu entry is removed.

- [ ] **Step 5.4: Verify and commit**

Run:

```bash
npm run build -w editors/vscode
npm run test:integration -w editors/vscode
```

**Expected:** The extension activates and exposes the documented history entry.

Commit:

```bash
git add editors/vscode/package.json editors/vscode/README.md editors/vscode/src/test/suite/extension.integration.ts
git commit -m "docs(vscode): explain repository operations and history"
```

---

## Task 6: Full product verification

**Context:** The work crosses a shared viewer, generated embedded assets, a VS Code
webview, native tree providers, and extension packaging. Focused tests alone do not
prove the shipped VSIX contains matching assets.

**Task goal:** Verify the complete extension as it will be packaged.

**Expected outcome:**

- All JavaScript unit tests and type checks pass.
- Viewer source and embedded assets match.
- Viewer accessibility and parity browser tests pass.
- VS Code integration tests pass.
- Packaging produces a smoke-testable VSIX.
- No `graphify-out/` directory is created or modified.
- Only planned source, test, documentation, package metadata, and generated viewer
  assets appear in the Compass repository diff.

- [ ] **Step 6.1: Run workspace tests and type checks**

**Step context:** This catches cross-package API mismatches after focused work.

**Step goal:** Prove TypeScript and unit behavior are consistent across workspaces.

Run:

```bash
npm run test:js
npm run typecheck:js
```

**Expected:** Every workspace test and type check passes.

- [ ] **Step 6.2: Validate generated viewer assets**

**Step context:** The extension copies CSS from `crates/compass-output/assets/viewer`
during its build.

**Step goal:** Prevent source/embedded asset drift.

Run:

```bash
npm run build -w @compass/viewer
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
```

**Expected:** The manifest hashes match generated JavaScript and CSS.

- [ ] **Step 6.3: Run browser and extension-host tests**

**Step context:** Accessibility interactions and command contribution require real
browser/extension environments.

**Step goal:** Verify user-visible behavior beyond unit boundaries.

Run:

```bash
npx playwright test tests/viewer/accessibility.spec.ts tests/viewer/graph-parity.spec.ts
npm run test:integration -w editors/vscode
```

**Expected:** Inspector interactions, graph parity, command registration, and
history contribution pass.

- [ ] **Step 6.4: Package and smoke-test the VSIX**

**Step context:** The packaged artifact is the deliverable users install.

**Step goal:** Confirm production bundling includes every required local asset.

Run:

```bash
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
```

**Expected:** Packaging creates a VSIX and smoke validation confirms required
commands, bundles, and local webview assets.

- [ ] **Step 6.5: Audit final scope**

**Step context:** The Compass repository already contains unrelated untracked
directories that belong to the user.

**Step goal:** Keep the handoff limited to this feature.

Run:

```bash
git status --short
git diff --check
git diff --stat
```

**Expected:** `git diff --check` prints nothing; no unrelated file is staged or
modified; no `graphify-out/` path is created or changed.

- [ ] **Step 6.6: Commit remaining generated assets only if needed**

**Step context:** Asset regeneration may be unchanged after earlier task commits.

**Step goal:** Avoid empty commits while keeping the repository reproducible.

Run:

```bash
git add crates/compass-output/assets/viewer/graph.js crates/compass-output/assets/viewer/viewer.css crates/compass-output/assets/viewer/manifest.json
git diff --cached --quiet || git commit -m "chore(viewer): refresh embedded assets"
```

**Expected:** Generated assets are either already current or committed in one
focused commit.
