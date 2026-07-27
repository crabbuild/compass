# Universal VS Code Call Graphs Design

**Date:** 2026-07-26

**Status:** Approved

**Implementation root:** `/Users/haipingfu/graphify/compass`

## Purpose

Compass will open caller and callee graphs from the VS Code editor for every
language that already emits callable nodes and call relationships into the
structural graph. Program IR remains an optional source of richer evidence; it
will no longer determine whether a language can open a call graph.

The immediate failure motivating this design occurs in the Entire repository.
The cursor byte `1683` is inside
`cmd/entire/cli/auth/control_plane.go::ResolveControlPlaneTarget`, and
`graph.json` contains that Go function, its source range, and its call edges.
The generated `program.json` contains no Go module because Go is intentionally
outside the first Program IR provider set. The current VS Code command queries
only Program IR and consequently reports:

```text
no Program IR function matches cmd/entire/cli/auth/control_plane.go:1683
```

This is an integration mismatch, not a cursor-conversion failure or missing Go
structural extraction.

## Goals

- Open graph-only call graphs for every existing Compass language that emits
  callable nodes and `calls` relationships.
- Enrich those graphs with Program IR call sites, unresolved calls, and
  coverage when Program IR exists.
- Let users choose callers, callees, or both from the editor context menu and
  switch direction inside the graph.
- Preserve confidence, ambiguity, unresolved evidence, and incomplete
  coverage rather than presenting all language support as equally precise.
- Keep call-graph construction, limits, and merge semantics in the Compass
  Rust CLI.
- Provide specific empty, partial, stale, and failure states in VS Code.
- Teach the workflow through a VS Code walkthrough and concise documentation.

## Non-goals

- Implement Go SSA, compiler-grade project analysis, or full Program IR for
  every language as part of this change.
- Claim that all language extractors provide identical call-resolution
  precision.
- Move graph traversal or semantic merging into the VS Code extension.
- Depend on a language server or VS Code document-symbol provider to identify
  the root symbol.
- Infer a callable from a nearest preceding declaration when no source range
  contains the cursor.
- Change general code-graph extraction semantics unrelated to callable ranges
  or call relationships.
- Run `graphify update`; this work is scoped to the Compass repository and the
  user explicitly requested that the Graphify updater not run.

## Existing support boundary

Compass currently has two distinct evidence layers:

1. The structural graph supports Go and many other languages. It records
   callable nodes, source lines, and resolved or inferred `calls` edges.
2. Program IR initially covered Rust and TypeScript/JavaScript, with Python
   added later. It records exact byte anchors, operations, unresolved calls,
   control-flow coverage, and evidence identities.

The existing call-graph builder accepts Program IR as its required root model
and consults `graph.json` only to refine confidence for Program IR edges. This
design makes the structural graph the universal baseline and Program IR an
optional enrichment layer.

## Architecture

The call-graph path has three explicit layers:

```text
structural graph ── universal callable nodes and call edges ──┐
                                                              ├─ Compass call-graph builder
Program IR ── optional exact operations and coverage ─────────┘
                                                                          │
                                                                          ▼
                                                        versioned call-graph response
                                                                          │
                                                                          ▼
                                                            VS Code call-graph viewer
```

The Rust builder owns:

- source-position and symbol-root resolution;
- structural and Program IR indexes;
- evidence-layer joining and deduplication;
- direction-aware bounded traversal;
- continuation calculation;
- deterministic ordering;
- coverage and limitation reporting; and
- the versioned response contract.

The extension owns:

- active-editor and repository selection;
- cursor byte and line calculation;
- command invocation, cancellation, and logging;
- response validation;
- native context-menu and Command Palette integration;
- webview lifecycle and typed messages; and
- source navigation.

The shared viewer owns graph presentation, direction controls, loading,
partial/empty/error states, inspection, and accessibility.

## Language-neutral CLI contract

Add a top-level machine-oriented command:

```text
compass call-graph \
  --file <REPOSITORY_RELATIVE_PATH> \
  --byte <UTF8_BYTE_OFFSET> \
  --line <ONE_BASED_LINE> \
  --direction <callers|callees|both> \
  --depth <N> \
  --max-nodes <N> \
  --max-edges <N> \
  --graph <PATH> \
  [--program <PATH>] \
  --format json
```

Separate `--file`, `--byte`, and `--line` arguments avoid ambiguous
`FILE:POSITION` parsing on Windows. Editor requests provide both positions:
Program IR uses the byte offset, while structural nodes use the line.

The command also accepts `--symbol <ID>` instead of a source position for lazy
expansion. A symbol ID may be a structural graph node ID or a Program IR symbol
ID associated with a structural node.

The existing `compass program call-graph` contract remains compatible for
existing consumers. It continues to produce the current Program IR response.
The VS Code extension moves to the new language-neutral command after
capability negotiation confirms support.

The new command returns a new schema rather than silently changing the meaning
of the Program IR schema. The response contains:

- root identity and source location;
- requested direction and depth;
- bounded nodes, edges, and continuations;
- structural graph IDs and optional Program IR symbol IDs;
- edge resolution and original confidence;
- available call-site evidence;
- evidence layer: `structural_graph`, `program_ir`, or `combined`;
- per-layer and per-language coverage limitations; and
- truncation and artifact freshness information.

Unknown major schemas fail before rendering. Additive compatible fields within
the supported major version are tolerated.

## Callable source-position contract

A structural call-graph root is a graph node that:

- has the requested normalized `source_file`;
- has a callable `symbol_kind`;
- has numeric one-based `line_start` and `line_end`; and
- contains the requested line inclusively.

Callable kinds include functions, methods, constructors, procedures, and
language-specific callable equivalents already represented by Compass. The
builder centralizes this classification so the extension does not maintain a
second language table.

When several callable nodes contain the cursor, Compass selects the smallest
line span, then the most specific callable kind, then the stable node ID. This
selects nested functions and methods deterministically.

Every extractor that emits a `calls` relationship must emit a usable inclusive
source range for its callable endpoints. Contract tests enforce this invariant.
An extractor that cannot provide a containing range reports missing root
coverage; Compass does not guess based on declaration order.

Program IR root resolution continues to use the smallest containing half-open
UTF-8 byte span. When both layers resolve, they must join through
`FunctionIr.graph_node_id`. A disagreement is surfaced as partial coverage and
does not silently replace the structural root.

## Universal graph construction

The builder indexes structural callable nodes by ID and source file and indexes
all `relation == "calls"` edges by source and target. It maps structural edge
confidence without inflating precision:

- `EXTRACTED` remains directly extracted structural evidence;
- `INFERRED` remains inferred;
- explicit ambiguous evidence remains ambiguous; and
- missing structural call evidence remains unknown rather than proving no
  call exists.

For graph-only languages, the graph supplies the complete available baseline.
Program IR is optional. If a matching Program IR function exists, Compass joins
it to the structural node through `graph_node_id` and adds:

- exact call-site byte anchors;
- resolved Program IR symbols;
- ambiguous and unresolved call nodes;
- evidence IDs; and
- capability coverage.

Overlapping structural and Program IR edges are deduplicated by structural
endpoint identity plus call-site identity. Program IR contributes exact
call-site and resolution details; a structural `INFERRED` confidence may still
downgrade an otherwise resolved relationship. Neither layer discards
limitations recorded by the other.

Traversal is breadth-first, deterministic, and direction-aware:

- callers follow incoming `calls` edges;
- callees follow outgoing `calls` edges; and
- both follows both sets without duplicating nodes or edges.

Depth, node, and edge limits remain positive and explicit. Excluded frontier
symbols become continuations. Sorting happens before bounds are applied.
Cancellation stops index or traversal work without publishing a partial
response as complete.

## VS Code interaction

The editor context menu contributes a native **Compass Call Graph** submenu
when the active resource is a file in a trusted workspace. It contains:

1. **Show Callers**
2. **Show Callees**
3. **Show Callers & Callees**

The same three actions appear in the Command Palette with `Compass:` prefixes.
The Compass sidebar's existing **Call graph from cursor** item opens both
directions by default.

Each action captures the active editor and selection before awaiting other UI,
selects the containing repository session, checks the language-neutral
call-graph capability, and opens the call-graph panel with the chosen
direction.

The graph toolbar contains a three-button direction control:

```text
[ Callers ] [ Both ] [ Callees ]
```

The active option has selected-state semantics beyond color. Activating another
option cancels the previous request and reloads the same root with the new
direction. Keyboard focus, the root indicator, and accessible status
announcements remain stable.

Lazy expansion continues to request callers, callees, or both for a selected
symbol. Expansion never changes the persistent root or the toolbar's root
direction.

## Guidance

Add a **Trace callers and callees** step to the existing Compass walkthrough.
It instructs users to:

1. Open a source file and place the cursor inside a function or method.
2. Right-click and open **Compass Call Graph**.
3. Choose callers, callees, or both.
4. Use the graph's direction buttons to change the view.
5. Select or open graph nodes to inspect and navigate to source.

The extension README and `docs/guides/vscode.md` use the same terminology and
explain:

- callers are functions that invoke the root;
- callees are functions invoked by the root;
- inferred and incomplete results are visibly qualified; and
- a Compass graph must be current before editor call graphs reflect recent
  source changes.

## Loading, empty, partial, and error states

The panel distinguishes these outcomes:

- **Loading:** resolving the active callable, tracing the chosen direction, and
  preparing evidence.
- **Empty callers or callees:** a valid root with no available relationships in
  that direction. This is not an error and does not claim complete absence when
  coverage is partial.
- **Partial graph:** a usable result with inferred relationships, incomplete
  extractor coverage, graph-only evidence, or truncation. The limitation is
  visible beside the graph.
- **Cursor outside a callable:** instruct the user to place the cursor inside a
  function or method.
- **Missing graph:** offer **Initialize**.
- **Stale graph:** offer **Update Graph** while allowing an existing safe graph
  to remain viewable when repository policy permits it.
- **Missing callable ranges or call coverage:** identify the language or
  construct limitation and offer **Show Compass Output**.
- **Artifact/schema/process failure:** retain **Retry** and **Show Compass
  Output**.

The raw Program IR message `no Program IR function matches ...` is not shown
when a valid structural callable exists. Output logs include repository,
relative file, byte, line, direction, selected evidence layer, and the
underlying typed failure without logging source contents.

## Freshness and compatibility

The request uses `graph.json` and optional `program.json` from the same
repository session. When both artifacts declare build identities, the builder
requires compatible identities before combining them. An incompatible Program
IR artifact is excluded with an explicit coverage warning; it is never joined
to a different structural graph.

Older Compass binaries continue to work with the existing Program IR command
for languages it supports. The new context-menu commands are disabled with
upgrade guidance when the CLI does not advertise the language-neutral
call-graph contract.

Existing graph artifacts work when they already contain callable ranges and
call edges. Users must update artifacts for extractors whose callable-range
contract is added by this change.

## Security and performance

- Paths remain repository-relative and are validated against the selected
  session root.
- The CLI receives argument arrays and is never invoked through a shell.
- Workspace trust remains mandatory.
- Graph and Program IR artifacts are size-bounded and validated before use.
- The webview receives only validated versioned payloads.
- Index construction is linear in relevant nodes and edges; traversal is
  bounded by explicit depth, node, and edge limits.
- Per-panel requests are cancellable, and superseded direction requests cannot
  overwrite newer results.
- No runtime network request or telemetry is introduced.

## Testing

### Rust analysis and CLI

- Add a graph-only Go fixture reproducing the Entire root and its direct calls.
- Verify callers-only, callees-only, and both traversal.
- Verify source-line resolution for functions, methods, nested callables,
  overlapping ranges, Unicode files, and normalized Windows paths.
- Verify structural confidence mapping and preservation.
- Verify Program IR enrichment through `graph_node_id`.
- Verify overlap deduplication and disagreement coverage.
- Verify ambiguous and unresolved Program IR calls survive merging.
- Verify deterministic ordering, depth, node, edge, and continuation bounds.
- Verify valid empty graphs and incomplete coverage.
- Verify artifact identity mismatch and cancellation behavior.
- Preserve tests for the legacy `program call-graph` command.
- Add end-to-end tests for the new top-level CLI arguments and schema.

### Extractor contracts

- For every language fixture that emits `calls`, assert both callable endpoints
  exist.
- Assert callable endpoints have normalized source files, callable kinds, and
  inclusive numeric source ranges.
- Assert every call edge retains explicit confidence.
- Add focused extractor tests where existing source-driven implementations need
  improved ranges.

### VS Code extension

- Verify the manifest contributes the submenu and all three commands.
- Verify each command captures and forwards file, byte, line, and direction.
- Verify the sidebar action defaults to both.
- Verify capability gating and upgrade guidance.
- Verify cancellation and stale-response suppression when direction changes.
- Verify initialization, update, retry, and output actions.

### Shared viewer and browser

- Verify the active direction's selected state, keyboard interaction, and
  accessible name.
- Verify switching direction sends the correct typed message.
- Verify loading, empty, partial, truncated, and error presentation.
- Verify structural-only and combined evidence labels.
- Verify source navigation remains available from nodes and call sites.
- Exercise light, dark, high-contrast, reduced-motion, and narrow layouts.

### Qualification

Run focused tests during red-green development, followed by:

```text
cargo fmt --all --check
cargo test for the affected Rust crates and CLI contracts
cargo clippy for the affected Rust crates with warnings denied
viewer tests and build
VS Code extension tests, typecheck, build, integration tests, package, and VSIX smoke test
```

Do not run `graphify update`.

## Acceptance criteria

1. In the Entire repository, right-clicking inside
   `ResolveControlPlaneTarget` can open its callers, callees, or both without a
   Program IR match.
2. The callee view includes the structurally extracted calls to
   `activeContext` and `targetForContext` with their original confidence.
3. Existing Program IR languages retain their richer call-site, unresolved,
   and coverage evidence.
4. Every language that emits callable structural call relationships can use the
   same CLI and VS Code path.
5. Languages or constructs with incomplete evidence receive explicit partial
   coverage rather than a false empty or generic Program IR error.
6. The editor context submenu, Command Palette actions, graph direction
   controls, walkthrough, and documentation use consistent caller/callee
   terminology.
7. Bounds, cancellation, schema validation, workspace trust, path safety, and
   offline behavior remain enforced.
8. Focused and qualification tests pass without running `graphify update`.
