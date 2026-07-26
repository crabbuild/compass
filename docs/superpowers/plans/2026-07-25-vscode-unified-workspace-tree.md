# VS Code Unified Workspace Tree Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the duplicate Repository and Operations sidebar views with one minimal, state-aware, VS Code-native Workspace tree.

**Architecture:** Preserve the stable `compass.status` view identifier and render its full contents from one pure `buildWorkspaceTree(discovery, sessions)` function. A single `WorkspaceTree` provider converts descriptors to native tree items, while existing command handlers remain authoritative for trust, capability, repository selection, and execution.

**Tech Stack:** TypeScript, VS Code Extension API, Vitest, Mocha VS Code extension-host tests, esbuild, VSCE.

## Global Constraints

- Implement directly; do not use a red-green TDD sequence.
- Add or update automated coverage after each implementation task.
- Keep the stable `compass.status` view identifier and rename its visible title to `Workspace`.
- Remove the `compass.operations` view contribution and provider.
- Render the sidebar exclusively with native VS Code tree items and `ThemeIcon` symbols.
- Do not add custom colors, webview markup, cards, badges, or decorative separators.
- List every workflow once in the Workspace tree.
- Keep existing command handlers and command-time trust, capability, and graph-state checks authoritative.
- Preserve active-editor repository resolution and the existing picker for ambiguous multi-root actions.
- `compass.refreshWorkspace` must refresh presentation state without starting a writer or watch process.
- Preserve unrelated user changes and untracked directories.
- Run `graphify update .` after code changes.

## File Map

- Modify `editors/vscode/src/views/treeModel.ts`: own the unified pure Workspace descriptor builder, state labels, action descriptors, and duplicate-command-free ordering.
- Modify `editors/vscode/src/views/treeModel.test.ts`: cover the unified tree after implementation.
- Create `editors/vscode/src/views/workspaceTree.ts`: provide the single native Workspace tree.
- Delete `editors/vscode/src/views/statusTree.ts`: superseded by `WorkspaceTree`.
- Delete `editors/vscode/src/views/operationsTree.ts`: remove the second provider.
- Modify `editors/vscode/src/extension.ts`: register one provider and a read-only Workspace refresh command.
- Modify `editors/vscode/src/commands/buildCommands.ts`: use the active editor repository before showing a multi-root picker, matching the shared selection contract.
- Modify `editors/vscode/package.json`: contribute one view, one title action, and the new refresh command.
- Modify `editors/vscode/src/test/suite/extension.integration.ts`: verify one contributed view and command registration.
- Modify `editors/vscode/README.md`: document the single Workspace information architecture.

---

### Task 1: Build the Unified Workspace Descriptor Model

**Files:**
- Modify: `editors/vscode/src/views/treeModel.ts`
- Modify after implementation: `editors/vscode/src/views/treeModel.test.ts`

**Interfaces:**
- Consumes: `CompassDiscovery`, `SessionTreeSnapshot`, existing Compass command identifiers.
- Produces:

```ts
export function buildWorkspaceTree(
  discovery: CompassDiscovery,
  sessions: readonly SessionTreeSnapshot[]
): TreeNode[];
```

- Preserves: `TreeNode`, `SessionTreeSnapshot`, `actionNode`, `graphStateLabel`, and `graphStateIcon`.
- Removes: `buildRepositoryTree`, `buildOperationsTree`, and repository routine-action children.

- [ ] **Step 1: Replace the two builders with the unified state model**

Implement `buildWorkspaceTree` with this top-level order:

```ts
const nodes: TreeNode[] = [
  ...cliAttentionNodes(discovery, sessions),
  ...repositoryStatusNodes(sessions),
  ...activeOperationNodes(sessions),
  ...recoveryNodes(sessions),
  ...exploreNodes(discovery, sessions),
  ...maintainNodes(discovery, sessions)
];
```

Repository descriptors have no routine workflow children:

```ts
{
  id: `repository:${session.id}`,
  label: path.basename(session.root) || session.root,
  description: graphStateLabel(session.graphState),
  tooltip: session.root,
  icon: graphStateIcon(session.graphState)
}
```

Use the canonical state labels:

```ts
if (state === "available") return "Graph ready";
if (state === "not-materialized") return "Not initialized";
if (state === "building") return "Building";
return "Build failed";
```

If there are no sessions, return one native action:

```ts
actionNode(
  "workspace:open-folder",
  "Open a repository folder",
  "folder-opened",
  "vscode.openFolder",
  "Open a folder to use Compass"
)
```

If the CLI is missing or any session is incompatible, render the attention row
and repository status rows, then omit Active operations, recovery, Explore, and
Maintain until setup is resolved.

- [ ] **Step 2: Implement conditional Active operations**

Render `Active operations` only when at least one writer or watcher exists.
Keep it expanded and use the repository basename as the child description:

```ts
{
  id: "workspace:active",
  label: "Active operations",
  description: String(active.length),
  icon: "pulse",
  expanded: true,
  children: active
}
```

Writer and watch rows are status-only. Writer rows use `sync~spin`; watcher rows
use `eye`. Start/stop control remains solely in Maintain so no command appears
twice.

- [ ] **Step 3: Implement state-specific recovery and unique workflow groups**

Render at most one global Initialize repository action when any repository is
not materialized, and at most one Retry graph build action when any repository
has failed:

```ts
actionNode(
  "workspace:initialize",
  "Initialize repository",
  "rocket",
  "compass.initialize",
  "Build the first Compass graph"
)
```

```ts
actionNode(
  "workspace:retry",
  "Retry graph build",
  "refresh",
  "compass.update",
  "Retry a failed Compass graph build"
)
```

Create one expanded Explore group. When at least one graph is available, its
children are ordered exactly:

```ts
[
  ["Code graph", "type-hierarchy", "compass.openGraph"],
  ["Architecture flow", "circuit-board", "compass.openArchitecture"],
  ["Call graph from cursor", "references", "compass.openCallGraph"],
  ["Ask codebase", "search", "compass.openQuery"],
  ["Codebase evolution", "history", "compass.openHistory"]
]
```

When no graph is available, retain only Codebase evolution.

Create one collapsed Maintain group only when a graph is available or a watch is
active. Its children are Update graph followed by either Watch for changes or
Stop watching. Do not include Initialize or Retry inside Maintain because their
state-specific top-level actions already exist.

- [ ] **Step 4: Add post-implementation tree-model coverage**

Replace the two old describe blocks with `describe("buildWorkspaceTree", ...)`.
Cover:

1. Healthy single repository:
   - top-level labels are `repo`, `Explore`, `Maintain`;
   - repository has no children;
   - each Compass workflow command appears once;
   - Explore is expanded and Maintain is collapsed.
2. Active writer and watcher:
   - Active operations appears between repository status and Explore;
   - child descriptions identify the repository;
   - Stop watching replaces Watch for changes.
3. Missing graph:
   - repository says Not initialized;
   - Initialize repository appears once;
   - Explore contains only Codebase evolution;
   - Maintain is absent.
4. Failed graph:
   - repository says Build failed;
   - Retry graph build appears once.
5. Missing and incompatible CLI:
   - setup row is first;
   - normal workflow groups are absent.
6. No repository:
   - Open a repository folder executes `vscode.openFolder`.
7. Multi-root:
   - all repository status rows appear;
   - only one Explore and one Maintain group exist;
   - no command identifier occurs more than once across the full tree snapshot.

Use a recursive command collector:

```ts
function commands(nodes: readonly TreeNode[]): string[] {
  return nodes.flatMap((node) => [
    ...(node.command ? [node.command] : []),
    ...commands(node.children ?? [])
  ]);
}
```

- [ ] **Step 5: Run focused model checks**

Run:

```bash
npm test -w editors/vscode -- src/views/treeModel.test.ts
npm run typecheck -w editors/vscode
```

Expected: all Workspace tree tests pass and TypeScript reports no errors.

- [ ] **Step 6: Commit the model**

```bash
git add editors/vscode/src/views/treeModel.ts editors/vscode/src/views/treeModel.test.ts
git commit -m "feat(vscode): unify workspace tree model"
```

---

### Task 2: Replace the Two Providers and Contributions

**Files:**
- Create: `editors/vscode/src/views/workspaceTree.ts`
- Delete: `editors/vscode/src/views/statusTree.ts`
- Delete: `editors/vscode/src/views/operationsTree.ts`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/src/commands/buildCommands.ts`
- Modify: `editors/vscode/package.json`
- Modify after implementation: `editors/vscode/src/test/suite/extension.integration.ts`

**Interfaces:**
- Consumes: `buildWorkspaceTree(discovery, registry.all())`.
- Produces:

```ts
export class WorkspaceTree implements vscode.TreeDataProvider<TreeNode> {
  readonly onDidChangeTreeData: vscode.Event<void>;
  refresh(): void;
  getTreeItem(node: TreeNode): vscode.TreeItem;
  getChildren(node?: TreeNode): TreeNode[];
}
```

- Adds command: `compass.refreshWorkspace`.
- Removes contributed view: `compass.operations`.
- Restricts Initialize and Update repository pickers to relevant graph states.

- [ ] **Step 1: Create the single Workspace provider**

Create `workspaceTree.ts` using the existing provider pattern:

```ts
export class WorkspaceTree implements vscode.TreeDataProvider<TreeNode> {
  private readonly changes = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changes.event;

  constructor(
    private readonly registry: SessionRegistry,
    private readonly discovery: CompassDiscovery
  ) {}

  refresh(): void {
    this.changes.fire();
  }

  getTreeItem(node: TreeNode): vscode.TreeItem {
    return treeItemFromNode(node);
  }

  getChildren(node?: TreeNode): TreeNode[] {
    if (node) return node.children ?? [];
    return buildWorkspaceTree(this.discovery, this.registry.all());
  }
}
```

Delete the superseded `statusTree.ts` and `operationsTree.ts`.

- [ ] **Step 2: Register one provider and a read-only refresh command**

In `extension.ts`:

- replace `StatusTree` and `OperationsTree` imports with `WorkspaceTree`;
- create one `workspaceTree`;
- update the shared refresh callback to call `workspaceTree.refresh()` and
  `statusBar.refresh()`;
- register only `compass.status`;
- register `compass.refreshWorkspace` to call the same read-only refresh callback.

The refresh command must not execute `compass.initialize`, `compass.update`, or
`compass.toggleWatch`.

- [ ] **Step 3: Align multi-root build selection**

In `pickRepository` inside `buildCommands.ts`, insert active-editor resolution
after explicit ID resolution and before single-session/picker fallback:

```ts
const editor = vscode.window.activeTextEditor;
const fromEditor = editor ? registry.forEditor(editor) : undefined;
if (fromEditor) return fromEditor;
```

This gives maintenance and recovery actions the same context resolution as
Explore commands. Extend `pickRepository` with an optional state predicate and
use it to limit Initialize to `not-materialized` repositories and Update to
`available` or `failed` repositories. Apply explicit ID, active editor, only
candidate, and filtered picker resolution in that order.

- [ ] **Step 4: Contribute one view and one title command**

In `editors/vscode/package.json`:

1. Add the command:

```json
{
  "command": "compass.refreshWorkspace",
  "title": "Compass: Refresh Status",
  "icon": "$(refresh)"
}
```

2. Replace the `views.compass` array with:

```json
[
  {
    "id": "compass.status",
    "name": "Workspace"
  }
]
```

3. Replace the Compass `view/title` entries with only:

```json
{
  "command": "compass.refreshWorkspace",
  "when": "view == compass.status",
  "group": "navigation"
}
```

Do not remove the existing primary workflow command contributions; they remain
available in the Command Palette.

- [ ] **Step 5: Add post-implementation integration assertions**

Update `extension.integration.ts` to assert:

```ts
assert.ok(commands.has("compass.refreshWorkspace"));
assert.deepEqual(
  extension.packageJSON.contributes.views.compass.map((view: { id: string }) => view.id),
  ["compass.status"]
);
```

Verify the single title menu entry is `compass.refreshWorkspace` and remove the
old assertion that Codebase Evolution appears in the Repository title.

- [ ] **Step 6: Run extension verification**

Run:

```bash
npm test -w editors/vscode
npm run typecheck -w editors/vscode
npm run build -w editors/vscode
```

Expected: all extension tests pass, TypeScript reports no errors, and esbuild
produces the extension/webview bundles.

- [ ] **Step 7: Commit the provider and contribution change**

```bash
git add editors/vscode/package.json \
  editors/vscode/src/extension.ts \
  editors/vscode/src/commands/buildCommands.ts \
  editors/vscode/src/views/workspaceTree.ts \
  editors/vscode/src/views/statusTree.ts \
  editors/vscode/src/views/operationsTree.ts \
  editors/vscode/src/test/suite/extension.integration.ts
git commit -m "feat(vscode): consolidate Compass workspace view"
```

---

### Task 3: Update the User-Facing Documentation

**Files:**
- Modify: `editors/vscode/README.md`

**Interfaces:**
- Consumes: canonical labels and state behavior from Tasks 1–2.
- Produces: one documented sidebar location for every Compass workflow.

- [ ] **Step 1: Replace Repository and Operations documentation**

Replace both sections with `### Workspace` and explain:

- repository rows report Graph ready, Not initialized, Building, or Build failed;
- Active operations appears only during builds or watching;
- Explore contains Code graph, Architecture flow, Call graph from cursor,
  Ask codebase, and Codebase evolution;
- Maintain contains Update graph and Watch for changes/Stop watching;
- Initialize repository and Retry graph build appear only when relevant;
- the title refresh action reads state and does not build a graph.

- [ ] **Step 2: Remove duplicate navigation claims**

Change the Git history introduction from:

```text
Open Codebase evolution from Repository, Operations, the Repository title bar,
or the Command Palette.
```

to:

```text
Open Codebase evolution from Workspace > Explore or the Command Palette.
```

Search for and remove obsolete mentions of separate Repository and Operations
views:

```bash
rg -n "Repository|Operations|Repository title bar" editors/vscode/README.md
```

Retain domain uses of “repository” that refer to actual repositories rather than
the removed view title.

- [ ] **Step 3: Commit the documentation**

```bash
git add editors/vscode/README.md
git commit -m "docs(vscode): document unified workspace view"
```

---

### Task 4: Final Verification and Repository Refresh

**Files:**
- Verify all changed files.
- Refresh: `/Users/haipingfu/graphify/graphify-out/`

**Interfaces:**
- Consumes: completed Tasks 1–3.
- Produces: a verified extension package and current Graphify knowledge graph.

- [ ] **Step 1: Inspect the complete change**

Run:

```bash
git status --short
git diff --check
git diff origin/main...HEAD --stat
```

Expected: no whitespace errors; unrelated untracked directories remain untouched.

- [ ] **Step 2: Run the complete JavaScript test suite**

Run:

```bash
npm run test:js
```

Expected: viewer, extension, and Playwright suites pass with zero failures.

- [ ] **Step 3: Run all JavaScript type checks and builds**

Run:

```bash
npm run typecheck:js
npm run build
```

Expected: all workspace type checks and production builds succeed.

- [ ] **Step 4: Package and smoke-test the VSIX**

Run:

```bash
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
```

Expected: VSCE creates `editors/vscode/compass-vscode-0.1.0.vsix` and the smoke
script reports success.

- [ ] **Step 5: Refresh Graphify metadata**

Run from `/Users/haipingfu/graphify`:

```bash
graphify update .
```

Expected: the AST-only knowledge graph refresh completes without an API call.

- [ ] **Step 6: Confirm final repository state**

Run:

```bash
git status --short
git log -5 --oneline --decorate
```

Expected: only pre-existing unrelated untracked directories remain; all owned
changes are committed.
