# Compass VS Code Foundation and Current Graph Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish the shared React viewer, machine-readable Compass bridge contracts, first-party VS Code extension shell, guided current-tree workflows, and an exact-parity current graph with source navigation.

**Architecture:** Rust owns graph preparation, command semantics, and safe HTML assembly. `packages/compass-viewer` owns React presentation and graph interaction; `editors/vscode` owns CLI processes, repositories, and webview transport. The exported graph and VS Code graph consume the same versioned `GraphViewModel` and React components.

**Tech Stack:** Rust 1.97.1, React, TypeScript, Vite, Tailwind CSS, shadcn/ui source components, Lucide, vis-network, Zod, Vitest, React Testing Library, esbuild, VS Code Extension API

## Global Constraints

- Implementation occurs in the standalone `compass` repository.
- The separately installed `compass` CLI is required; do not bundle native binaries.
- Do not read Compass history SQLite or Prolly internals from TypeScript.
- Use `spawn` with argument arrays and `shell: false`; never build shell command strings.
- Use VS Code workspace trust and run beside the repository on the extension host.
- Remote SSH, WSL, and Dev Containers must locate and run Compass on the remote extension host; browser-only `vscode.dev` is unsupported.
- Exported HTML and VS Code use the same viewer model, reducer, components, and interaction fixtures.
- Bundle every webview/export dependency locally; no runtime CDN, remote font, or telemetry.
- Do not add telemetry events, identifiers, or network reporting.
- Preserve the current 5,000-node overview threshold.
- Map shadcn/Tailwind tokens to VS Code CSS variables and support light, dark, high-contrast, and reduced motion.
- Treat IDs, labels, metadata, and source paths as untrusted.
- Run `graphify update .` from the Compass root after every task that changes Rust or TypeScript code.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `package.json`, `package-lock.json`, `tsconfig.base.json` | Root npm workspace, pinned dependency graph, and shared TypeScript settings. |
| `packages/compass-viewer/src/contracts/graph.ts` | Runtime and TypeScript `GraphViewModel` contract. |
| `packages/compass-viewer/src/graph/state.ts` | Pure graph reducer and interaction commands. |
| `packages/compass-viewer/src/graph/CompassGraph.tsx` | Shared graph workspace, toolbar, search, filters, and inspector. |
| `packages/compass-viewer/src/graph/VisNetworkCanvas.tsx` | vis-network adapter only. |
| `packages/compass-viewer/src/components/ui/*` | Minimal shadcn/ui source components used by the viewer. |
| `packages/compass-viewer/src/theme.css` | Tailwind entry and VS Code/export semantic tokens. |
| `packages/compass-viewer/src/export-entry.tsx` | Self-mounting exported-HTML entry. |
| `crates/compass-output/src/viewer_model.rs` | Authoritative Rust construction of `GraphViewModel`. |
| `crates/compass-output/src/html.rs` | Offline HTML shell embedding shared viewer assets and model. |
| `crates/compass-output/assets/viewer/*` | Deterministically built viewer JavaScript, CSS, and manifest. |
| `crates/compass-cli/src/ide_contract.rs` | Capability and progress-event schemas. |
| `crates/compass-cli/src/capability_commands.rs` | `compass capabilities --format json`. |
| `editors/vscode/src/cli/*` | Discovery, capability negotiation, process lifecycle, and argument builders. |
| `editors/vscode/src/workspace/*` | Repository discovery, selection, session state, and artifact freshness. |
| `editors/vscode/src/views/*` | Activity-bar status, setup, operations, and graph panel hosts. |
| `editors/vscode/src/webviews/graph.tsx` | VS Code graph webview entry using `CompassGraph`. |
| `editors/vscode/src/transport/messages.ts` | Validated host/webview message schemas. |

### Task 1: Create the JavaScript workspace and a tested viewer contract

**Files:**
- Create: `package.json`
- Create: `package-lock.json`
- Create: `tsconfig.base.json`
- Create: `packages/compass-viewer/package.json`
- Create: `packages/compass-viewer/tsconfig.json`
- Create: `packages/compass-viewer/vite.config.ts`
- Create: `packages/compass-viewer/src/contracts/graph.ts`
- Create: `packages/compass-viewer/src/contracts/graph.test.ts`
- Create: `packages/compass-viewer/src/index.ts`

**Interfaces:**
- Produces: `GraphViewModelSchema`, `GraphViewModel`, `GraphNode`, `GraphEdge`, and `GRAPH_VIEWER_SCHEMA = "compass.viewer.graph/1"`.
- Consumes: no application code.

- [ ] **Step 1: Create the workspace manifests and install the locked toolchain**

Use this root manifest:

```json
{
  "name": "compass-workspace",
  "private": true,
  "workspaces": ["packages/*", "editors/*", "tests/*"],
  "scripts": {
    "build:viewer": "npm run build -w @compass/viewer",
    "test:js": "npm run test --workspaces --if-present",
    "typecheck:js": "npm run typecheck --workspaces --if-present"
  }
}
```

Use this viewer package manifest before installing workspace dependencies:

```json
{
  "name": "@compass/viewer",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "exports": { ".": "./src/index.ts" },
  "scripts": {
    "build": "vite build",
    "test": "vitest run",
    "typecheck": "tsc --noEmit"
  }
}
```

Run:

```bash
npm install --save-dev typescript vite vitest jsdom @vitejs/plugin-react tailwindcss @tailwindcss/vite
npm install -w packages/compass-viewer react react-dom zod vis-network lucide-react class-variance-authority clsx tailwind-merge
npm install --save-dev -w packages/compass-viewer @types/react @types/react-dom @testing-library/react @testing-library/user-event
```

Set `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `moduleResolution: "Bundler"`, `jsx: "react-jsx"`, and `target: "ES2022"` in `tsconfig.base.json`. Commit the generated root `package-lock.json`.

- [ ] **Step 2: Write the failing viewer-contract test**

```ts
import { describe, expect, it } from "vitest";
import { GraphViewModelSchema } from "./graph";

describe("GraphViewModelSchema", () => {
  it("accepts additive fields but rejects the wrong major schema", () => {
    const base = {
      schema: "compass.viewer.graph/1",
      title: "Fixture",
      stats: { nodes: 1, edges: 0, communities: 1, aggregated: false },
      nodes: [{ id: "n1", label: "run", community: 0, future: true }],
      edges: [],
      communities: [{ id: 0, label: "Core", color: "#4f8cff", hidden: false }],
      hyperedges: [],
      future: "preserved"
    };
    expect(GraphViewModelSchema.parse(base).future).toBe("preserved");
    expect(() => GraphViewModelSchema.parse({ ...base, schema: "compass.viewer.graph/2" }))
      .toThrow();
  });
});
```

- [ ] **Step 3: Run the test and verify it fails**

Run: `npm test -w @compass/viewer -- --run src/contracts/graph.test.ts`

Expected: FAIL because `GraphViewModelSchema` does not exist.

- [ ] **Step 4: Implement the runtime contract**

```ts
import { z } from "zod";

export const GRAPH_VIEWER_SCHEMA = "compass.viewer.graph/1" as const;
const SourceSchema = z.object({
  file: z.string(),
  startLine: z.number().int().positive().optional(),
  endLine: z.number().int().positive().optional(),
  startByte: z.number().int().nonnegative().optional(),
  endByte: z.number().int().nonnegative().optional()
}).passthrough();
export const GraphNodeSchema = z.object({
  id: z.string().min(1),
  label: z.string(),
  kind: z.string().optional(),
  community: z.number().int(),
  communityName: z.string().optional(),
  degree: z.number().int().nonnegative().optional(),
  source: SourceSchema.optional()
}).passthrough();
export const GraphEdgeSchema = z.object({
  id: z.string().min(1),
  source: z.string().min(1),
  target: z.string().min(1),
  relation: z.string(),
  confidence: z.enum(["extracted", "inferred", "ambiguous"]).optional()
}).passthrough();
export const GraphViewModelSchema = z.object({
  schema: z.literal(GRAPH_VIEWER_SCHEMA),
  title: z.string(),
  stats: z.object({
    nodes: z.number().int().nonnegative(),
    edges: z.number().int().nonnegative(),
    communities: z.number().int().nonnegative(),
    aggregated: z.boolean()
  }).passthrough(),
  nodes: z.array(GraphNodeSchema),
  edges: z.array(GraphEdgeSchema),
  communities: z.array(z.object({
    id: z.number().int(), label: z.string(), color: z.string(), hidden: z.boolean()
  }).passthrough()),
  hyperedges: z.array(z.unknown())
}).passthrough();
export type GraphViewModel = z.infer<typeof GraphViewModelSchema>;
export type GraphNode = z.infer<typeof GraphNodeSchema>;
export type GraphEdge = z.infer<typeof GraphEdgeSchema>;
```

- [ ] **Step 5: Verify, update the code graph, and commit**

Run:

```bash
npm test -w @compass/viewer -- --run src/contracts/graph.test.ts
npm run typecheck:js
graphify update .
git add package.json package-lock.json tsconfig.base.json packages/compass-viewer
git commit -m "build(viewer): establish shared React workspace"
```

Expected: the contract test and typecheck pass.

### Task 2: Add the authoritative Rust graph-view model and JSON export

**Files:**
- Create: `crates/compass-output/src/viewer_model.rs`
- Modify: `crates/compass-output/src/lib.rs`
- Modify: `crates/compass-output/src/html.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Create: `crates/compass-cli/tests/viewer_export_cli.rs`

**Interfaces:**
- Consumes: `GraphDocument`, `Communities`, `HtmlOptions`, and the existing node/edge preparation logic in `html.rs`.
- Produces: `GRAPH_VIEWER_SCHEMA`, `GraphViewModel`, `graph_view_model(...)`, and `compass export viewer-json`.

- [ ] **Step 1: Write the failing CLI contract**

```rust
#[test]
fn viewer_json_matches_the_html_view_model() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = support::fixture_graph()?;
    let output = support::run_compass_in(
        fixture.path(),
        ["export", "viewer-json", "--graph", "compass-out/graph.json"],
    );
    assert_eq!(output.code, 0, "{}", output.stderr);
    let model: serde_json::Value = serde_json::from_str(&output.stdout)?;
    assert_eq!(model["schema"], "compass.viewer.graph/1");
    assert_eq!(model["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(model["edges"][0]["relation"], "calls");
    assert_eq!(model["edges"][0]["source"], model["nodes"][0]["id"]);
    Ok(())
}
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run: `cargo test -p compass-cli --test viewer_export_cli`

Expected: FAIL with unknown export format `viewer-json`.

- [ ] **Step 3: Extract a typed model without changing current HTML behavior**

Define:

```rust
pub const GRAPH_VIEWER_SCHEMA: &str = "compass.viewer.graph/1";

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphViewModel {
    pub schema: &'static str,
    pub title: String,
    pub stats: GraphViewStats,
    pub nodes: Vec<serde_json::Value>,
    pub edges: Vec<serde_json::Value>,
    pub communities: Vec<CommunityView>,
    pub hyperedges: Vec<serde_json::Value>,
}

pub fn graph_view_model(
    document: &GraphDocument,
    communities: &Communities,
    title: &str,
    options: &HtmlOptions<'_>,
) -> Result<GraphViewModel, OutputError>;
```

Move the current node, edge, legend/community, aggregation, and hyperedge preparation behind this function. Preserve stable IDs, community colors, learning-overlay fields, sanitization, and the 5,000-node aggregation rule.

- [ ] **Step 4: Add `compass export viewer-json`**

Route `viewer-json` through the existing export input loader. It accepts `--graph`, `--labels`, and `--node-limit`, writes pretty JSON to stdout by default, and uses existing atomic output when `--output PATH` is supplied. Update `help.rs` with the exact command.

- [ ] **Step 5: Verify parity and commit**

Run:

```bash
cargo fmt --all
cargo test -p compass-output
cargo test -p compass-cli --test viewer_export_cli
graphify update .
git add crates/compass-output crates/compass-cli/src/lib.rs crates/compass-cli/src/help.rs crates/compass-cli/tests/viewer_export_cli.rs
git commit -m "feat(output): expose shared graph viewer model"
```

Expected: existing HTML security/parity tests and the new JSON contract pass.

### Task 3: Implement the shared graph state and accessible React workspace

**Files:**
- Create: `packages/compass-viewer/src/graph/state.ts`
- Create: `packages/compass-viewer/src/graph/state.test.ts`
- Create: `packages/compass-viewer/src/graph/CompassGraph.tsx`
- Create: `packages/compass-viewer/src/graph/CompassGraph.test.tsx`
- Create: `packages/compass-viewer/src/graph/VisNetworkCanvas.tsx`
- Create: `packages/compass-viewer/src/components/ui/button.tsx`
- Create: `packages/compass-viewer/src/components/ui/input.tsx`
- Create: `packages/compass-viewer/src/components/ui/scroll-area.tsx`
- Create: `packages/compass-viewer/src/lib/cn.ts`
- Create: `packages/compass-viewer/src/theme.css`
- Modify: `packages/compass-viewer/src/index.ts`

**Interfaces:**
- Consumes: `GraphViewModel`.
- Produces: `GraphState`, `graphReducer`, `CompassGraph`, `GraphHost`, and `GraphCanvasAdapter`.

- [ ] **Step 1: Write reducer tests for the exact Compass interaction rules**

```ts
it("focus pauses physics and clearing focus does not resume it", () => {
  const focused = graphReducer(initialGraphState, { type: "focus", nodeId: "n1" });
  expect(focused).toMatchObject({ focusedNodeId: "n1", physicsRunning: false });
  const cleared = graphReducer(focused, { type: "clearFocus" });
  expect(cleared).toMatchObject({ focusedNodeId: null, physicsRunning: false });
});

it("stabilization pauses once and explicit resume is the only restart", () => {
  const paused = graphReducer(initialGraphState, { type: "stabilized" });
  expect(paused.physicsRunning).toBe(false);
  expect(graphReducer(paused, { type: "setPhysics", running: true }).physicsRunning).toBe(true);
});
```

- [ ] **Step 2: Run the reducer test and verify it fails**

Run: `npm test -w @compass/viewer -- --run src/graph/state.test.ts`

Expected: FAIL because `graphReducer` is missing.

- [ ] **Step 3: Implement the pure state boundary**

```ts
export type GraphState = {
  focusedNodeId: string | null;
  physicsRunning: boolean;
  forceLabels: boolean;
  hiddenCommunities: ReadonlySet<number>;
  query: string;
};
export type GraphAction =
  | { type: "focus"; nodeId: string }
  | { type: "clearFocus" }
  | { type: "stabilized" }
  | { type: "setPhysics"; running: boolean }
  | { type: "setLabels"; visible: boolean }
  | { type: "toggleCommunity"; communityId: number }
  | { type: "search"; query: string };
```

Use immutable `Set` replacement. Never store a vis-network instance in reducer state.

- [ ] **Step 4: Write the component interaction test**

```tsx
it("routes canvas, search, and neighbor choices through one focus action", async () => {
  const host = { openSource: vi.fn() };
  render(<CompassGraph model={fixtureModel} host={host} canvas={fakeCanvas} />);
  await userEvent.type(screen.getByRole("searchbox"), "helper");
  await userEvent.click(screen.getByRole("option", { name: /helper/i }));
  expect(screen.getByRole("status")).toHaveTextContent("Inspecting helper");
  await userEvent.click(screen.getByRole("button", { name: /caller run/i }));
  expect(screen.getByRole("status")).toHaveTextContent("Inspecting run");
  await userEvent.click(screen.getByRole("button", { name: /open source/i }));
  expect(host.openSource).toHaveBeenCalledWith(expect.objectContaining({ file: "src/lib.rs" }));
});
```

- [ ] **Step 5: Implement the component and vis-network adapter**

`CompassGraph` owns semantic UI and sends adapter commands. `VisNetworkCanvas` owns construction, event subscription, spotlight style updates, stabilization callbacks, focus animation, fit/reset, and disposal. Use real buttons, listbox/option semantics, a polite live region, and `prefers-reduced-motion`.

Map CSS tokens to VS Code variables with export fallbacks:

```css
@import "tailwindcss";
:root {
  --background: var(--vscode-editor-background, #101722);
  --foreground: var(--vscode-editor-foreground, #d8e2ef);
  --border: var(--vscode-panel-border, #314055);
  --accent: var(--vscode-focusBorder, #4f8cff);
  --destructive: var(--vscode-errorForeground, #f87171);
}
```

- [ ] **Step 6: Verify, update the graph, and commit**

Run:

```bash
npm test -w @compass/viewer -- --run src/graph
npm run typecheck:js
graphify update .
git add packages/compass-viewer
git commit -m "feat(viewer): add shared Compass graph workspace"
```

### Task 4: Embed the shared viewer in offline `graph.html`

**Files:**
- Create: `packages/compass-viewer/src/export-entry.tsx`
- Modify: `packages/compass-viewer/vite.config.ts`
- Create: `scripts/build_viewer_assets.mjs`
- Create: `crates/compass-output/assets/viewer/manifest.json`
- Create: `crates/compass-output/assets/viewer/graph.js`
- Create: `crates/compass-output/assets/viewer/graph.css`
- Modify: `crates/compass-output/src/html.rs`
- Modify: `crates/compass-output/tests/coverage_paths.rs`

**Interfaces:**
- Consumes: `GraphViewModel`, `CompassGraph`.
- Produces: `mountExportedGraph(root, model)` and a self-contained HTML shell.

- [ ] **Step 1: Add a failing offline/shared-bundle Rust test**

```rust
#[test]
fn graph_html_uses_the_shared_offline_viewer() -> Result<(), Box<dyn Error>> {
    let rendered = fixture_html()?;
    for marker in [
        "compass.viewer.graph/1",
        "id=\"compass-viewer-root\"",
        "id=\"compass-viewer-model\"",
        "data-viewer-build=",
        "default-src 'none'",
    ] {
        assert!(rendered.html.contains(marker), "missing {marker}");
    }
    assert!(!rendered.html.contains("cdn.jsdelivr.net"));
    assert!(!rendered.html.contains("<script src=\"http"));
    Ok(())
}
```

- [ ] **Step 2: Build a deterministic IIFE and manifest**

`export-entry.tsx` reads JSON from `#compass-viewer-model`, validates it, and mounts `CompassGraph`. Configure Vite for one IIFE named `CompassViewer`, deterministic filenames, no source map in published assets, and extracted CSS. `build_viewer_assets.mjs` writes a manifest containing SHA-256 for both assets and fails if either output contains `http://` or `https://`.

- [ ] **Step 3: Replace only the HTML presentation shell**

Keep Rust graph-model preparation. Emit:

```html
<main id="compass-viewer-root"></main>
<script id="compass-viewer-model" type="application/json">...</script>
<script nonce="...">/* locally embedded graph.js */</script>
```

Escape `<`, `>`, `&`, U+2028, and U+2029 in the JSON script payload. Inline the checked CSS and JS after verifying their hashes against `manifest.json` in a test.

Replace renderer tests that search for private hand-written JavaScript function
names with model/shell contracts. Preserve security tests in Rust and move
focus, stabilization, search, neighbor navigation, and reduced-motion behavior
to the shared React/real-browser fixtures; do not keep two presentation
implementations merely to satisfy old string assertions.

- [ ] **Step 4: Run parity and security tests**

Run:

```bash
npm run build:viewer
cargo test -p compass-output
cargo test -p compass-parity
npm test -w @compass/viewer
```

Expected: current raw-node/raw-edge parity, script-terminator security, stable-layout behavior, and viewer component tests pass.

- [ ] **Step 5: Update the graph and commit**

Run:

```bash
graphify update .
git add packages/compass-viewer scripts/build_viewer_assets.mjs crates/compass-output
git commit -m "feat(output): share the React graph viewer"
```

### Task 5: Add CLI capability and progress-event contracts

**Files:**
- Create: `crates/compass-cli/src/ide_contract.rs`
- Create: `crates/compass-cli/src/capability_commands.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/bin/compass.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/src/init_commands.rs`
- Create: `crates/compass-cli/tests/capabilities_cli.rs`
- Create: `crates/compass-cli/tests/progress_events_cli.rs`

**Interfaces:**
- Produces: `CapabilityReport`, `ProgressEvent`, `ProgressState`, `ProgressWriter`, `compass capabilities --format json`, and `--events jsonl` for guided operations.
- Consumes: schema constants from Compass crates and `GRAPH_VIEWER_SCHEMA`.

- [ ] **Step 1: Write failing capability and terminal-event tests**

```rust
#[test]
fn capabilities_reports_versioned_ide_contracts() -> Result<(), Box<dyn Error>> {
    let output = run(Frontend::Compass, arguments(["capabilities", "--format", "json"]));
    assert_eq!(output.code, 0, "{}", output.stderr);
    let value: Value = serde_json::from_str(&output.stdout)?;
    assert_eq!(value["schema"], "compass.ide.capabilities/1");
    assert_eq!(value["contracts"]["graph_viewer"], "compass.viewer.graph/1");
    assert!(value["compass_version"].is_string());
    Ok(())
}

#[test]
fn machine_update_emits_exactly_one_terminal_event() -> Result<(), Box<dyn Error>> {
    let output = run_fixture(["update", ".", "--no-viz", "--events", "jsonl"])?;
    let events = output.stdout.lines().map(serde_json::from_str::<Value>).collect::<Result<Vec<_>, _>>()?;
    assert_eq!(events.iter().filter(|e| e["terminal"] == true).count(), 1);
    assert_eq!(events.last().and_then(|e| e["state"].as_str()), Some("succeeded"));
    Ok(())
}
```

- [ ] **Step 2: Define the serializable contracts**

```rust
pub const CAPABILITY_SCHEMA: &str = "compass.ide.capabilities/1";
pub const PROGRESS_SCHEMA: &str = "compass.ide.progress/1";

#[derive(Serialize)]
pub struct CapabilityReport {
    pub schema: &'static str,
    pub compass_version: &'static str,
    pub contracts: BTreeMap<&'static str, &'static str>,
    pub features: BTreeMap<&'static str, bool>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressState { Started, Running, Retrying, Succeeded, Failed, Cancelled }

#[derive(Serialize)]
pub struct ProgressEvent<'a> {
    pub schema: &'static str,
    pub operation_id: &'a str,
    pub operation: &'a str,
    pub state: ProgressState,
    pub phase: &'a str,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub message: &'a str,
    pub terminal: bool,
}
```

`ProgressWriter<W: Write>` writes one compact JSON object plus newline and flushes. It prevents a second terminal event.

- [ ] **Step 3: Wire capabilities and machine events**

Add `capabilities` to native dispatch and help. Parse `--events jsonl` before normal `init`, `update`, and `watch` options. Convert existing build/watch observer statuses to progress events. Human output remains unchanged without the flag. A broken progress stream returns exit code `1`.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo fmt --all
cargo test -p compass-cli --test capabilities_cli --test progress_events_cli
cargo test -p compass-cli --test init_cli --test update_cli --test watch_cli
graphify update .
git add crates/compass-cli
git commit -m "feat(cli): add IDE capability and progress contracts"
```

### Task 6: Scaffold the first-party VS Code extension and safe process host

**Files:**
- Create: `editors/vscode/package.json`
- Create: `editors/vscode/tsconfig.json`
- Create: `editors/vscode/esbuild.mjs`
- Create: `editors/vscode/src/extension.ts`
- Create: `editors/vscode/src/cli/discovery.ts`
- Create: `editors/vscode/src/cli/processManager.ts`
- Create: `editors/vscode/src/cli/contracts.ts`
- Create: `editors/vscode/src/workspace/repositorySession.ts`
- Create: `editors/vscode/src/workspace/sessionRegistry.ts`
- Create: `editors/vscode/src/test/discovery.test.ts`
- Create: `editors/vscode/src/test/processManager.test.ts`
- Create: `editors/vscode/media/compass.svg`
- Create: `editors/vscode/LICENSE`
- Modify: `package-lock.json`

**Interfaces:**
- Consumes: `compass capabilities --format json` and JSONL progress.
- Produces: `discoverCompass()`, `CompassProcessManager`, `RepositorySession`, and `SessionRegistry`.

- [ ] **Step 1: Add the extension manifest and Lucide Compass activity icon**

Create the package as `"name": "compass-vscode"`,
`"displayName": "Compass"`, `"publisher": "crabbuild"`,
`"version": "0.1.0"`, and `"private": true`; the npm workspace name is
`@compass/vscode`. Use extension kind `workspace`, VS Code engine `^1.95.0`,
activation on `onStartupFinished`, a `compass` activity container, commands
`compass.initialize`, `compass.update`, `compass.toggleWatch`, and
`compass.openGraph`, and machine-scoped `compass.cliPath`. Add scripts
`build`, `test`, `typecheck`, `test:integration`, `package`, and `smoke:vsix`
with `dist/extension.js` as `main`.

Use the Lucide Compass paths in a monochrome `24×24` SVG with `currentColor`; record the Lucide ISC license in `LICENSE`/notices.

- [ ] **Step 2: Write process-safety tests**

```ts
it("never invokes a shell and preserves hostile arguments", async () => {
  const spawn = vi.fn().mockReturnValue(fakeChild({ stdout: "{}\n", code: 0 }));
  const manager = new CompassProcessManager("/tmp/compass", spawn);
  await manager.run("/repo", ["query", "$(touch /tmp/pwned)", "--format", "json"]);
  expect(spawn).toHaveBeenCalledWith(
    "/tmp/compass",
    ["query", "$(touch /tmp/pwned)", "--format", "json"],
    expect.objectContaining({ cwd: "/repo", shell: false, windowsHide: true })
  );
});
```

- [ ] **Step 3: Implement discovery and process lifecycle**

```ts
export type CommandResult = { code: number; stdout: string; stderr: string };
export class CompassProcessManager {
  run(cwd: string, args: readonly string[], signal?: AbortSignal): Promise<CommandResult>;
  runJson<T>(cwd: string, args: readonly string[], schema: z.ZodType<T>, signal?: AbortSignal): Promise<T>;
  startJsonl(cwd: string, args: readonly string[], onEvent: (event: ProgressEvent) => void): RunningCommand;
}
export type RunningCommand = { operationId: string; completed: Promise<CommandResult>; cancel(): void };
```

Bound captured stdout/stderr at 8 MiB for ordinary commands. JSONL mode parses line-by-line, rejects an unknown major schema, requires exactly one terminal event, and keeps raw lines in the output channel.

- [ ] **Step 4: Implement per-repository sessions**

`SessionRegistry` discovers Git/Compass repositories per workspace folder, never chooses an ambiguous nested repository for writes, and maps the active editor to a remembered repository. `RepositorySession` owns capability state, graph paths, freshness, active writer, and watch process.

- [ ] **Step 5: Verify and commit**

Run:

```bash
npm test -w @compass/vscode -- --run src/test/discovery.test.ts src/test/processManager.test.ts
npm run typecheck:js
npm run build -w @compass/vscode
graphify update .
git add package.json package-lock.json editors/vscode
git commit -m "feat(vscode): add Compass extension process host"
```

### Task 7: Add guided setup, init, update, watch, and native status views

**Files:**
- Create: `editors/vscode/src/commands/buildCommands.ts`
- Create: `editors/vscode/src/commands/buildArguments.ts`
- Create: `editors/vscode/src/commands/buildArguments.test.ts`
- Create: `editors/vscode/src/views/setupView.ts`
- Create: `editors/vscode/src/views/statusTree.ts`
- Create: `editors/vscode/src/views/operationsTree.ts`
- Create: `editors/vscode/src/views/statusBar.ts`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/package.json`

**Interfaces:**
- Consumes: `RepositorySession`, `CompassProcessManager`.
- Produces: `buildInitArgs`, `buildUpdateArgs`, `buildWatchArgs`, and registered guided commands.

- [ ] **Step 1: Write exact argument-builder tests**

```ts
expect(buildInitArgs({
  root: "/repo", includes: ["src/**"], excludes: ["vendor/**"], force: false
})).toEqual([
  "init", "/repo", "--include", "src/**", "--exclude", "vendor/**",
  "--yes", "--events", "jsonl"
]);
expect(buildWatchArgs({ root: "/repo", debounceSeconds: 0.4, poll: true }))
  .toEqual(["watch", "/repo", "--debounce", "0.4", "--poll", "--events", "jsonl"]);
```

- [ ] **Step 2: Implement guided forms with native VS Code controls**

Use `showOpenDialog`, `showInputBox`, `showQuickPick`, `withProgress`, and tree commands. Require an explicit repository before writing. Preview init includes/excludes before confirmation. Do not invoke an installer automatically; missing CLI view provides official commands and `Select Compass Binary`.

- [ ] **Step 3: Coordinate writer and watch state**

Only one writer may run per repository. Updating while watch is active offers to stop watch first. Stopping watch uses `RunningCommand.cancel()`. Progress events update `OperationsTree`, status bar, and output channel; routine intermediate events do not create notifications.

- [ ] **Step 4: Add stale-artifact refresh**

Watch atomic replacements of `compass-out/graph.json`, `program.json`, `manifest.json`, and `.compass/config.toml`. Debounce 250 ms, validate file existence after replacement, and refresh status only after a successful command terminal event or complete published artifact set.

- [ ] **Step 5: Verify and commit**

Run:

```bash
npm test -w @compass/vscode -- --run src/commands
npm run typecheck:js
npm run build -w @compass/vscode
graphify update .
git add editors/vscode
git commit -m "feat(vscode): guide Compass setup and graph builds"
```

### Task 8: Ship the current graph webview and source navigation

**Files:**
- Create: `editors/vscode/src/transport/messages.ts`
- Create: `editors/vscode/src/transport/messages.test.ts`
- Create: `editors/vscode/src/views/graphPanel.ts`
- Create: `editors/vscode/src/views/sourceNavigation.ts`
- Create: `editors/vscode/src/views/sourceNavigation.test.ts`
- Create: `editors/vscode/src/webviews/graph.tsx`
- Modify: `editors/vscode/esbuild.mjs`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/package.json`

**Interfaces:**
- Consumes: `compass export viewer-json`, `GraphViewModelSchema`, and `CompassGraph`.
- Produces: `GraphPanel`, `HostToGraphMessageSchema`, `GraphToHostMessageSchema`, and `openGraphSource`.

- [ ] **Step 1: Write transport and path-boundary tests**

```ts
it("rejects a source-open message for another repository", () => {
  expect(() => GraphToHostMessageSchema.parse({
    type: "openSource", repositoryId: "other", source: { file: "../../secret" }
  })).not.toThrow();
  expect(resolveSource(activeRepository, "other", "../../secret")).toEqual({
    kind: "repository-mismatch"
  });
});
```

The runtime schema accepts the untrusted message; the host authorization layer rejects repository mismatch and paths outside the repository unless the user explicitly confirms the external target.

- [ ] **Step 2: Implement typed graph hydration**

`GraphPanel` waits for `{type:"ready"}`, runs:

```text
compass export viewer-json --graph <absolute graph.json> --format json
```

validates `GraphViewModelSchema`, then sends `{type:"hydrateGraph", requestId, repositoryId, model}`. A newer request cancels the old one. Webview CSP permits only the extension-local bundle and VS Code resource roots.

- [ ] **Step 3: Implement source opening**

Resolve `source.file` against the repository root, reject traversal/symlink escape, open with `workspace.openTextDocument`, and reveal the most precise available line or UTF-8 byte range. For byte offsets, decode the document buffer and convert through `TextDocument.positionAt`.

- [ ] **Step 4: Add extension-host smoke coverage**

Create a fixture workspace, point `compass.cliPath` to a deterministic fake CLI, open the graph command, assert the webview receives schema `compass.viewer.graph/1`, send an `openSource` message, and assert the editor reveals `src/lib.rs`.

- [ ] **Step 5: Run the milestone gate and commit**

Run:

```bash
npm run build:viewer
npm test -w @compass/viewer
npm test -w @compass/vscode
npm run typecheck:js
cargo test -p compass-output -p compass-cli
graphify update .
git add editors/vscode packages/compass-viewer crates/compass-output/assets/viewer
git commit -m "feat(vscode): explore the Compass graph in editor"
```

Expected: a trusted fixture workspace can initialize/update, open the shared graph, focus/search/filter nodes, and reveal source without a shell.
