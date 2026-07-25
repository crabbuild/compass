# Compass VS Code Calls, Architecture Flow, and Query Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a cursor-rooted, evidence-aware symbol call graph, a shared architecture call-flow document, and purpose-built natural-language and CompassQL experiences.

**Architecture:** Compass derives versioned call and architecture models from validated Program IR and graph data. The extension resolves editor positions, invokes structured CLI commands, and streams validated models into shared React views. Query semantics remain entirely in existing Compass query engines.

**Tech Stack:** Rust, Compass Program IR, React, TypeScript, Zod, Tailwind CSS, shadcn/ui, Lucide, Vitest, React Testing Library, VS Code Webview API

## Global Constraints

- Complete the foundation/current-graph plan first.
- Preserve Program IR's rule that unresolved calls never prove absence.
- Distinguish resolved, inferred, ambiguous, and unresolved calls with text/shape as well as color.
- Resolve symbols by UTF-8 byte position and choose the innermost containing function.
- Call expansion is bounded, lazy, cancellable, and deterministic.
- Architecture flow uses one Rust-derived model and one React presentation in export and VS Code.
- No remote Mermaid script or runtime network request.
- Historical query selection is added in the evolution plan; this plan completes current-tree query UX.
- Run `graphify update .` after code changes.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/compass-analysis/src/call_graph.rs` | Deterministic Program IR call neighborhood and coverage model. |
| `crates/compass-cli/src/program_commands.rs` | `program call-graph` parsing and JSON rendering. |
| `crates/compass-output/src/callflow_model.rs` | Versioned architecture-flow presentation model. |
| `crates/compass-output/src/callflow.rs` | Offline shell using the shared architecture component. |
| `packages/compass-viewer/src/calls/*` | Call graph reducer, canvas, inspector, evidence, and coverage. |
| `packages/compass-viewer/src/architecture/*` | Architecture navigation, diagrams, call tables, and statistics. |
| `packages/compass-viewer/src/query/*` | Query mode, form state, structured results, and navigation. |
| `editors/vscode/src/views/callGraphPanel.ts` | Cursor resolution and lazy expansion host. |
| `editors/vscode/src/views/architecturePanel.ts` | Architecture model host. |
| `editors/vscode/src/views/queryPanel.ts` | Query execution and result host. |

### Task 1: Add a deterministic Program IR call-graph model

**Files:**
- Create: `crates/compass-analysis/src/call_graph.rs`
- Modify: `crates/compass-analysis/src/lib.rs`
- Modify: `crates/compass-analysis/Cargo.toml`
- Create: `crates/compass-analysis/tests/call_graph.rs`

**Interfaces:**
- Produces: `CallGraphRequest`, `CallGraphDirection`, `CallGraphResponse`, `CallNode`, `CallEdge`, `CallResolution`, `CallContinuation`, and `build_call_graph`.
- Consumes: `AnalysisBundle` and optional `GraphDocument` confidence evidence.

- [ ] **Step 1: Write failing direction, ambiguity, and unresolved tests**

```rust
#[test]
fn call_graph_preserves_resolution_and_bounds() -> Result<(), Box<dyn Error>> {
    let analysis = fixture_analysis_with_resolved_ambiguous_and_unresolved_calls()?;
    let response = build_call_graph(
        &analysis,
        None,
        &CallGraphRequest {
            root: CallGraphRoot::Symbol { symbol: "root".into() },
            direction: CallGraphDirection::Both,
            depth: 1,
            max_nodes: 4,
            max_edges: 4,
        },
    )?;
    assert_eq!(response.schema, "compass.program.call_graph/1");
    assert!(response.edges.iter().any(|edge| edge.resolution == CallResolution::Ambiguous));
    assert!(response.edges.iter().any(|edge| edge.resolution == CallResolution::Unresolved));
    assert!(response.truncated);
    assert!(!response.continuations.is_empty());
    Ok(())
}
```

- [ ] **Step 2: Implement the public model**

```rust
pub const CALL_GRAPH_SCHEMA: &str = "compass.program.call_graph/1";
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallGraphRoot { Symbol { symbol: String }, SourceByte { file: String, byte: u64 } }
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallGraphDirection { Callers, Callees, Both }
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallResolution { Resolved, Inferred, Ambiguous, Unresolved }
pub struct CallGraphRequest {
    pub root: CallGraphRoot, pub direction: CallGraphDirection, pub depth: u32,
    pub max_nodes: usize, pub max_edges: usize,
}
pub fn build_call_graph(
    analysis: &AnalysisBundle,
    graph: Option<&GraphDocument>,
    request: &CallGraphRequest,
) -> Result<CallGraphResponse, CallGraphError>;
```

Index functions and operations once. One `resolved_symbols` entry is resolved; multiple entries are ambiguous; zero is unresolved. When both endpoint `graph_node_id` values match a `calls` edge with `confidence=INFERRED`, classify the otherwise resolved edge as inferred. Preserve every call-site anchor and evidence ID. Sort by symbol/call-site identity before applying bounds.

- [ ] **Step 3: Resolve source roots and continuations**

For `SourceByte`, select functions whose anchor contains the byte and choose the smallest byte span, then symbol ID. Breadth-first traversal applies direction/depth and returns a continuation for every frontier symbol excluded by depth or size bounds.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo fmt --all
cargo test -p compass-analysis --test call_graph
graphify update .
git add crates/compass-analysis
git commit -m "feat(program): derive bounded symbol call graphs"
```

### Task 2: Expose `compass program call-graph`

**Files:**
- Modify: `crates/compass-cli/src/program_commands.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/src/capability_commands.rs`
- Modify: `crates/compass-cli/tests/program_cli.rs`

**Interfaces:**
- Consumes: `build_call_graph`.
- Produces: `compass program call-graph (--symbol ID|--at FILE:BYTE) [--direction callers|callees|both] [--depth N] [--max-nodes N] [--max-edges N] [--graph PATH] --format json`.

- [ ] **Step 1: Add the failing end-to-end CLI test**

```rust
let outcome = run(
    Frontend::Compass,
    arguments([
        "program", "call-graph", "--at", "src/lib.rs:28",
        "--direction", "both", "--depth", "2",
        "--program", program_arg, "--graph", graph_arg, "--format", "json",
    ]),
);
assert_eq!(outcome.code, 0, "{}", outcome.stderr);
let value: Value = serde_json::from_str(&outcome.stdout)?;
assert_eq!(value["schema"], "compass.program.call_graph/1");
assert_eq!(value["root_symbol"], helper_symbol);
```

- [ ] **Step 2: Implement strict parsing**

Require exactly one of `--symbol` and `--at`; reject zero depth, zero bounds, duplicate options, invalid UTF-8 byte syntax, and text output for this machine-first command. Load optional graph through the existing safe graph loader.

- [ ] **Step 3: Update help and capabilities**

Report `program_call_graph: "compass.program.call_graph/1"` in capabilities. Add examples for cursor byte, callers only, and bounded expansion.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo fmt --all
cargo test -p compass-cli --test program_cli --test capabilities_cli
graphify update .
git add crates/compass-cli
git commit -m "feat(cli): expose structured program call graphs"
```

### Task 3: Build the interactive shared call-graph view

**Files:**
- Create: `packages/compass-viewer/src/contracts/callGraph.ts`
- Create: `packages/compass-viewer/src/calls/state.ts`
- Create: `packages/compass-viewer/src/calls/state.test.ts`
- Create: `packages/compass-viewer/src/calls/CallGraph.tsx`
- Create: `packages/compass-viewer/src/calls/CallGraph.test.tsx`
- Create: `packages/compass-viewer/src/calls/CallCanvas.tsx`
- Create: `packages/compass-viewer/src/calls/CoverageNotice.tsx`
- Modify: `packages/compass-viewer/src/index.ts`

**Interfaces:**
- Consumes: `compass.program.call_graph/1`.
- Produces: `CallGraph`, `CallGraphHost`, `callGraphReducer`, and `mergeExpansion`.

- [ ] **Step 1: Test idempotent expansion and root breadcrumbs**

```ts
it("merges expansion without duplicating evidence and preserves the root", () => {
  const once = mergeExpansion(initial, expansion);
  const twice = mergeExpansion(once, expansion);
  expect(twice.nodes).toHaveLength(once.nodes.length);
  expect(twice.edges[0].callSites).toEqual(once.edges[0].callSites);
  expect(twice.rootSymbol).toBe("root");
});
```

- [ ] **Step 2: Implement runtime validation and reducer**

Use Zod discriminated unions for resolution and continuation. Keep `rootSymbol`, `selectedNode`, `expandedSymbols`, `direction`, nodes/edges by ID, pending continuation IDs, and coverage limitations in reducer state.

- [ ] **Step 3: Implement the accessible view**

Render directional arrows, dashed inferred edges, double-line ambiguous edges, and explicit unresolved terminal nodes. The inspector lists call-site anchors and evidence. Provide callers/callees/both controls, depth, expand/collapse, root reset, breadcrumbs, search, and source buttons.

- [ ] **Step 4: Verify and commit**

Run:

```bash
npm test -w @compass/viewer -- --run src/calls
npm run typecheck:js
graphify update .
git add packages/compass-viewer
git commit -m "feat(viewer): add evidence-aware call graph"
```

### Task 4: Connect the call graph to the active editor

**Files:**
- Create: `editors/vscode/src/views/callGraphPanel.ts`
- Create: `editors/vscode/src/views/cursorByte.ts`
- Create: `editors/vscode/src/views/cursorByte.test.ts`
- Create: `editors/vscode/src/webviews/callGraph.tsx`
- Modify: `editors/vscode/src/transport/messages.ts`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/package.json`

**Interfaces:**
- Consumes: `CallGraph`, `compass program call-graph`.
- Produces: `Compass: Show Call Graph`, editor/context-menu command, and lazy `expandCallGraph` transport.

- [ ] **Step 1: Test UTF-8 cursor conversion**

```ts
it("converts VS Code UTF-16 offsets to UTF-8 bytes", () => {
  const text = "fn café() { helper(); }\n";
  const offset = text.indexOf("helper");
  expect(utf8ByteOffset(text, offset)).toBe(Buffer.byteLength(text.slice(0, offset), "utf8"));
});
```

- [ ] **Step 2: Implement root resolution**

Use the active document's workspace repository and relative POSIX path. Send `--at <path>:<byte>`. If no function contains the cursor, show a symbol Quick Pick populated by `compass program functions --name ... --format json`.

- [ ] **Step 3: Implement cancellable expansion**

An expansion message sends `--symbol`, direction, depth `1`, and bounds. Abort it when the root changes or panel closes. Validate schema before posting. Open node and call-site sources through the existing authorized source-navigation service.

- [ ] **Step 4: Verify and commit**

Run:

```bash
npm test -w @compass/vscode -- --run src/views/cursorByte.test.ts
npm run build -w @compass/vscode
npm run typecheck:js
graphify update .
git add editors/vscode
git commit -m "feat(vscode): open call graphs from the cursor"
```

### Task 5: Create the shared architecture-flow model and React document

**Files:**
- Create: `crates/compass-output/src/callflow_model.rs`
- Modify: `crates/compass-output/src/callflow.rs`
- Modify: `crates/compass-output/src/lib.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Create: `crates/compass-output/tests/callflow_model.rs`
- Create: `packages/compass-viewer/src/contracts/callflow.ts`
- Create: `packages/compass-viewer/src/architecture/ArchitectureFlow.tsx`
- Create: `packages/compass-viewer/src/architecture/ArchitectureFlow.test.tsx`

**Interfaces:**
- Produces: `CALLFLOW_VIEWER_SCHEMA = "compass.viewer.callflow/1"`, Rust `CallflowViewModel`, `compass export callflow-json`, and React `ArchitectureFlow`.
- Consumes: existing section derivation, community labels, report highlights, graph evidence.

- [ ] **Step 1: Add the failing Rust model test**

```rust
let model = callflow_view_model(&document, &communities, &options)?;
assert_eq!(model.schema, "compass.viewer.callflow/1");
assert_eq!(model.sections[0].id, "overview");
assert!(model.sections.iter().all(|section| !section.nodes.is_empty() || section.id == "overview"));
assert!(serde_json::to_string(&model)?.contains("\"confidence\""));
```

- [ ] **Step 2: Refactor derivation into typed records**

Define overview links, sections, diagram nodes/edges, call rows, report highlights, hyperedges, statistics, provenance, and source anchors as serializable structs. Keep language selection and current ordering rules in Rust.

- [ ] **Step 3: Add JSON export and shared HTML mount**

Add `compass export callflow-json` with the same graph/labels/report/section options as `callflow-html`. Change `callflow-html` to embed the model and local architecture bundle rather than remote Mermaid.

- [ ] **Step 4: Implement React architecture navigation**

Render an accessible section nav, overview diagram, subsystem diagrams, call tables, confidence/evidence badges, highlights, and statistics. Diagram layout runs locally and exposes a table alternative.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo fmt --all
cargo test -p compass-output --test callflow_model
npm test -w @compass/viewer -- --run src/architecture
graphify update .
git add crates/compass-output crates/compass-cli packages/compass-viewer
git commit -m "feat(callflow): share structured architecture documents"
```

### Task 6: Add architecture and query editor tabs

**Files:**
- Create: `editors/vscode/src/views/architecturePanel.ts`
- Create: `editors/vscode/src/views/queryPanel.ts`
- Create: `editors/vscode/src/commands/queryArguments.ts`
- Create: `editors/vscode/src/commands/queryArguments.test.ts`
- Create: `editors/vscode/src/webviews/architecture.tsx`
- Create: `editors/vscode/src/webviews/query.tsx`
- Create: `packages/compass-viewer/src/query/QueryWorkspace.tsx`
- Create: `packages/compass-viewer/src/query/QueryWorkspace.test.tsx`
- Modify: `editors/vscode/src/transport/messages.ts`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/package.json`

**Interfaces:**
- Consumes: `compass export callflow-json`, natural-language `compass query`, CompassQL JSON.
- Produces: `Compass: Open Architecture Flow`, `Compass: Query`, and structured query result navigation.

- [ ] **Step 1: Test safe query arguments**

```ts
expect(buildCqlArgs({
  query: "MATCH (n) RETURN n LIMIT 5",
  params: { kind: "Function" },
  timeoutMs: 5000,
  maxRows: 100
})).toEqual([
  "query", "--cql", "MATCH (n) RETURN n LIMIT 5",
  "--param", "kind=Function", "--timeout-ms", "5000",
  "--max-rows", "100", "--format", "json"
]);
```

- [ ] **Step 2: Implement query state and rendering**

Provide explicit natural-language and CompassQL tabs, query history, parameters, limits, cancellation, table/path/raw views, and source/graph actions for recognized node and evidence cells. Keep history in workspace state and cap it at 100 query strings without storing results.

- [ ] **Step 3: Implement host execution**

Natural-language mode captures human output and recognized node references. CompassQL always requests JSON and validates `compass.cql.result/1`. Use `--output` only for explicit Save Result.

- [ ] **Step 4: Run the milestone gate and commit**

Run:

```bash
cargo test -p compass-analysis -p compass-output -p compass-cli
npm test -w @compass/viewer
npm test -w @compass/vscode
npm run typecheck:js
graphify update .
git add editors/vscode packages/compass-viewer
git commit -m "feat(vscode): add architecture and query workspaces"
```
