# Universal VS Code Call Graphs Implementation Plan

> **For agentic workers:** Execute this plan inline with
> `superpowers:executing-plans`. The tasks are organized around Compass
> ownership boundaries and independently reviewable deliverables. Tests are
> regression and contract verification, not a repetitive TDD script.

**Goal:** Make VS Code caller/callee graphs work for every Compass language
that already emits callable structural nodes and `calls` edges, while retaining
Program IR as optional enrichment.

**Architecture:** Add a language-neutral call-graph builder beside the existing
Program IR builder. The new builder treats `graph.json` as the required
baseline, joins optional Program IR through `graph_node_id`, and emits a new
versioned response consumed by a top-level `compass call-graph` command. VS Code
passes both byte and line cursor positions, exposes three native direction
commands, and renders direction controls through the shared viewer.

**Tech Stack:** Rust 1.97.1, Serde/JSON, Compass `GraphDocument`, Compass
Program IR, TypeScript, Zod, React, Vitest, VS Code Extension API

## Global Constraints

- Preserve `compass.program.call_graph/1` and the existing
  `compass program call-graph` behavior.
- Use `compass.call_graph/1` for the language-neutral contract.
- Keep traversal and merge semantics in Rust; the extension must not read or
  traverse graph artifacts.
- Structural results preserve `EXTRACTED`, `INFERRED`, and ambiguous evidence
  without upgrading confidence.
- Program IR is optional and may only enrich a structural graph through an
  exact `graph_node_id` join.
- Root resolution never guesses from the nearest declaration.
- All requests remain bounded, deterministic, cancellable at the process
  boundary, workspace-trusted, and repository-relative.
- Do not run `graphify update`.

---

## Current implementation context

The existing Program IR implementation is in
`crates/compass-analysis/src/call_graph.rs`. It:

- requires `AnalysisBundle`;
- resolves source roots only from half-open UTF-8 byte anchors;
- uses `graph.json` solely to downgrade matching Program IR edges to inferred;
  and
- emits `compass.program.call_graph/1`.

`crates/compass-cli/src/program_commands.rs` owns the legacy parser. The VS Code
host in `editors/vscode/src/views/callGraphPanel.ts` always invokes that legacy
command with `--at FILE:BYTE`, which is why graph-only Go files fail.

The shared viewer already renders the legacy response and lazy expansion.
`CallCanvas.tsx` can navigate byte-backed nodes but the common graph source
contract also supports line ranges. The new response can therefore add graph
line locations without changing source-navigation infrastructure.

The Entire artifact that motivated this work already contains:

- a Go callable node for `ResolveControlPlaneTarget` spanning lines 42–55;
- extracted calls to `activeContext` and `targetForContext`; and
- no matching Go module in `program.json`.

This plan fixes that integration boundary before expanding richer language
semantics.

## File structure

| File | Responsibility |
| --- | --- |
| `crates/compass-analysis/src/universal_call_graph.rs` | Graph-first root resolution, optional Program IR enrichment, bounded traversal, and `compass.call_graph/1` model. |
| `crates/compass-analysis/src/lib.rs` | Export the new model without changing legacy exports. |
| `crates/compass-analysis/tests/universal_call_graph.rs` | Graph-only Go, direction, root, confidence, merge, and bounds contracts. |
| `crates/compass-cli/src/call_graph_commands.rs` | Parse and execute top-level `compass call-graph`. |
| `crates/compass-cli/src/lib.rs` | Dispatch the new top-level command. |
| `crates/compass-cli/src/help.rs` | Root help page and examples. |
| `crates/compass-cli/src/capability_commands.rs` | Advertise `call_graph` contract and `call_graph` feature. |
| `crates/compass-cli/tests/call_graph_cli.rs` | End-to-end graph-only and combined command coverage. |
| `packages/compass-viewer/src/contracts/callGraph.ts` | Validate the new schema while retaining the legacy schema. |
| `packages/compass-viewer/src/calls/CallGraph.tsx` | Direction toolbar, evidence-layer badges, partial and empty states. |
| `packages/compass-viewer/src/calls/CallCanvas.tsx` | Map byte or line-backed nodes to common source locations. |
| `packages/compass-viewer/src/calls/state.ts` | Preserve universal response metadata during lazy expansion. |
| `packages/compass-viewer/src/calls/CallGraph.test.tsx` | Direction interaction and empty/partial rendering. |
| `editors/vscode/src/views/callGraphArguments.ts` | Pure argument construction for root, expansion, and direction changes. |
| `editors/vscode/src/views/callGraphArguments.test.ts` | Platform-neutral command argument contracts. |
| `editors/vscode/src/views/callGraphPanel.ts` | Capture root position/direction, invoke the new CLI, cancel superseded requests, and route direction changes. |
| `editors/vscode/src/webviews/callGraph.tsx` | Send direction changes and render new responses. |
| `editors/vscode/src/cli/compatibility.ts` | Require the language-neutral capability. |
| `editors/vscode/src/extension.ts` | Register callers, callees, both, and legacy-default commands. |
| `editors/vscode/package.json` | Contribute commands, submenu, editor menu, and walkthrough step. |
| `editors/vscode/src/views/treeModel.ts` | Keep the sidebar action as the both-directions default. |
| `editors/vscode/README.md` | Extension-facing workflow instructions. |
| `docs/guides/vscode.md` | Product documentation for the right-click workflow and coverage semantics. |

## Task 1: Add the graph-first Rust model

**Context**

The legacy Program IR structs are tightly shaped around byte anchors and must
remain stable. A separate model prevents an accidental compatibility break and
lets graph-only nodes carry line locations and evidence-layer metadata.

**Interfaces**

Create and export:

```rust
pub const UNIVERSAL_CALL_GRAPH_SCHEMA: &str = "compass.call_graph/1";

pub enum UniversalCallGraphRoot {
    Symbol { symbol: String },
    SourcePosition { file: String, byte: u64, line: u64 },
}

pub struct UniversalCallGraphRequest {
    pub root: UniversalCallGraphRoot,
    pub direction: CallGraphDirection,
    pub depth: u32,
    pub max_nodes: usize,
    pub max_edges: usize,
}

pub fn build_universal_call_graph(
    graph: &GraphDocument,
    analysis: Option<&AnalysisBundle>,
    request: &UniversalCallGraphRequest,
) -> Result<UniversalCallGraphResponse, UniversalCallGraphError>;
```

The response uses structural graph IDs as canonical node IDs. Program IR
symbols remain optional metadata. Unresolved Program IR-only nodes use stable
`unresolved:<caller>:<ordinal>` IDs.

**Implementation**

- Index callable nodes using normalized `source_file`, `symbol_kind`,
  `line_start`, and `line_end`.
- Treat `function`, `method`, `constructor`, `procedure`, and
  `subroutine` as callable kinds, plus graph nodes that are endpoints of a
  `calls` edge and have a non-file source range.
- Resolve the smallest containing range, then callable-kind rank, then node ID.
- Resolve `--symbol` by graph node ID first and optional Program IR symbol ID
  second.
- Convert structural `calls` edges to universal edges. Use the source node's
  file and the edge's `source_location` line as structural call-site evidence.
- Map `INFERRED` to inferred and explicit ambiguous confidence to ambiguous;
  all other structural calls remain resolved-with-structural-evidence.
- Join Program IR functions by `graph_node_id`. Add exact byte call sites,
  unresolved nodes, and ambiguity without duplicating structural endpoint
  relationships.
- Reuse the legacy breadth-first traversal semantics while keeping universal
  continuation symbols addressable by the new command.
- Report evidence layer (`structural_graph`, `program_ir`, or `combined`) and a
  dynamic coverage warning.

**Verification**

- A Go graph with no Program IR resolves a cursor inside lines 42–55 and
  returns both extracted callees.
- Incoming-only, outgoing-only, and both traversal produce exact expected
  nodes.
- Nested callable ranges select the smallest span.
- Inferred structural edges remain inferred.
- Program IR enriches matching graph nodes and adds unresolved evidence.
- Bounds and ordering are deterministic.
- `cargo test -p compass-analysis --test universal_call_graph`
- `cargo test -p compass-analysis`

**Commit:** `feat(analysis): add universal structural call graphs`

## Task 2: Expose `compass call-graph`

**Context**

The extension needs a command that does not load `program.json` before parsing.
The graph is required; Program IR is optional. Explicit artifact paths must
still fail clearly when unreadable or invalid.

**Interfaces**

`call_graph_commands::command(frontend, args)` accepts:

```text
--file PATH --byte N --line N | --symbol ID
--direction callers|callees|both
--depth N
--max-nodes N
--max-edges N
--graph PATH
[--program PATH]
--format json
```

Defaults are `compass-out/graph.json`, depth 2, 250 nodes, and 500 edges. Program
IR is absent unless `--program` is supplied explicitly, which prevents a custom
graph path from being enriched with an unrelated default artifact.

**Implementation**

- Add a focused command module rather than extending
  `program_commands.rs`.
- Validate root option exclusivity and require file, byte, and line together.
- Load and validate the graph before optional Program IR.
- Reuse canonical Program IR loading semantics; expose the loader
  `pub(crate)` or move the shared read/validation into the new command module
  without weakening existing checks.
- Add root dispatch, help, closest-command recognition, and capability
  advertisement:
  - contract `call_graph: compass.call_graph/1`
  - feature `call_graph: true`
- Preserve `program_call_graph: compass.program.call_graph/1`.

**Verification**

- A temporary Go repository updated by Compass can execute graph-only
  `call-graph` without a Go Program IR module.
- Separate file/byte/line arguments work with Unicode and colon-containing
  paths.
- Optional Program IR upgrades the evidence layer to combined.
- Invalid direction, bounds, missing root parts, unreadable graph, and invalid
  explicit Program IR return typed non-zero outcomes.
- Capability and help snapshots include both legacy and universal contracts.
- `cargo test -p compass-cli --test call_graph_cli`
- `cargo test -p compass-cli --test capabilities_cli`
- `cargo test -p compass-cli --test help_cli`

**Commit:** `feat(cli): expose language-neutral call graphs`

## Task 3: Upgrade the shared viewer contract and controls

**Context**

The viewer must validate both schemas during transition. The VS Code extension
will require the universal capability and render only universal responses, but
legacy exports and tests must continue compiling.

**Interfaces**

- Export `UniversalCallGraphResponseSchema` and
  `UniversalCallGraphResponse`.
- Export `AnyCallGraphResponseSchema` and `AnyCallGraphResponse` as the
  discriminated union used by common rendering.
- Extend `CallGraphHost` with:

```ts
changeDirection(direction: CallDirection): void;
```

**Implementation**

- Model universal node source as optional byte and line coordinates.
- Include `evidenceLayer` and coverage `partial`/`limitations`.
- Adapt `CallCanvas` to emit byte-backed source locations when present and
  line-backed locations otherwise.
- Add a top toolbar with Callers, Both, and Callees buttons using
  `aria-pressed`.
- Render an evidence-layer badge and a valid directional empty state.
- Show partial coverage whenever the response is graph-only, truncated,
  inferred, ambiguous, unresolved, or explicitly limited.
- Keep lazy expansion and root identity stable.

**Verification**

- Existing legacy state tests remain green.
- Universal contract fixtures parse and invalid schemas fail.
- Direction buttons send exactly one host callback and expose selected state.
- Graph-only empty results render as empty rather than error.
- Line-backed graph nodes map to navigable `SourceLocation`.
- `npm test --workspace @compass/viewer`
- `npm run build --workspace @compass/viewer`

**Commit:** `feat(viewer): add universal call graph controls`

## Task 4: Add native VS Code caller/callee actions

**Context**

The current panel hard-codes `both` and uses one abort controller for the panel
lifetime. Direction switches require request-scoped cancellation so a slow old
response cannot overwrite the newest direction.

**Interfaces**

Register:

```text
compass.openCallGraph            default both, retained for sidebar compatibility
compass.openCallers
compass.openCallees
compass.openCallersAndCallees
```

`CallGraphPanel.open` gains a `CallDirection` argument. A pure
`callGraphArguments.ts` module builds root and expansion arrays.

**Implementation**

- Capture `editor.selection.active`, convert to UTF-8 byte and
  `position.line + 1`, and retain the root for retries/direction changes.
- Invoke top-level `call-graph` with separate file, byte, and line args.
- Pass `--program` only when the artifact exists; always pass the session
  graph.
- Replace request controllers whenever root direction changes; preserve panel
  disposal cancellation.
- Add a `changeDirection` webview message and hydrate rather than merge its
  response.
- Contribute a native `compass.callGraph` submenu in `editor/context`.
- Place the three direction commands in the submenu and Command Palette.
- Keep the sidebar action routed to `compass.openCallGraph` and both direction.
- Require feature `call_graph` and contract `compass.call_graph/1`.
- Improve logs with file, byte, line, and direction.

**Verification**

- Argument tests cover each root direction, symbol expansion, Unicode byte
  offsets, and Windows-safe paths.
- Manifest tests or JSON assertions cover submenu contribution and commands.
- Compatibility tests reject old CLIs with actionable upgrade copy.
- Extension integration lists all registered commands.
- `npm test --workspace crabbuild-compass-vscode`
- `npm run typecheck --workspace crabbuild-compass-vscode`
- `npm run build --workspace crabbuild-compass-vscode`

**Commit:** `feat(vscode): add caller and callee context actions`

## Task 5: Add guidance and qualify the packaged extension

**Context**

The user requested editor instructions, not only discoverable commands. The
workflow must appear in the built-in VS Code walkthrough and in both extension
and product documentation.

**Implementation**

- Add walkthrough step `compass.calls` titled **Trace callers and callees**.
- Use the extension README as its local Markdown media.
- Document the exact right-click submenu sequence, direction meanings,
  direction toolbar, evidence qualifiers, graph freshness, and source
  navigation.
- Update the call-graph error copy so missing callable coverage is distinct
  from process/artifact failure.
- Build and package the VSIX after all focused checks pass.

**Verification**

- `cargo fmt --all --check`
- `cargo test -p compass-analysis`
- `cargo test -p compass-cli --test call_graph_cli --test program_cli --test capabilities_cli --test help_cli`
- `cargo clippy -p compass-analysis -p compass-cli --all-targets -- -D warnings`
- `npm test --workspace @compass/viewer`
- `npm run build --workspace @compass/viewer`
- `npm test --workspace crabbuild-compass-vscode`
- `npm run typecheck --workspace crabbuild-compass-vscode`
- `npm run build --workspace crabbuild-compass-vscode`
- `npm run package --workspace crabbuild-compass-vscode`
- `npm run smoke:vsix --workspace crabbuild-compass-vscode`
- Manually execute the built CLI against
  `/Users/haipingfu/Github/entire/compass-out/graph.json` at
  `cmd/entire/cli/auth/control_plane.go`, line 42, and confirm the two known
  callees are present.
- Confirm `graphify update` was not invoked.

**Commit:** `docs(vscode): explain caller and callee graphs`
