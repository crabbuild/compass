# VS Code Architecture Flow Implementation Plan

**Execution style:** Implementation-first with context. Implement each cohesive
slice, add focused regression coverage immediately afterward, verify it, and
commit it before starting the next slice. Do not use a red-green TDD sequence.

**Goal:** Load Django-sized architecture exports safely and present a complete,
directional, production-first architecture and call-flow workspace in VS Code.

**Architecture:** Compass emits a complete additive
`compass.viewer.callflow/1` model.
The extension host captures up to 128 MiB for this command, retains and indexes
the full model, and sends bounded projections to the webview. The shared viewer
renders those projections as an SVG subsystem map with paged evidence.

**Tech stack:** Rust, serde, TypeScript, Zod, React 19, Vitest, Testing Library,
SVG, VS Code webviews, existing Compass viewer CSS tokens.

**Approved design:** `docs/superpowers/specs/2026-07-28-vscode-architecture-flow-design.md`

## Global constraints

- Keep the default process stdout and stderr limit at 8 MiB.
- Allow 128 MiB stdout only for architecture export; keep its stderr at 8 MiB.
- Measure limits in UTF-8 bytes.
- Never post the complete call-flow model to the webview.
- Production is the initial scope; `All code` restores tests, generated, vendor,
  and unknown scopes without loss.
- Every edge is counted as internal, cross-section, or unassigned.
- Use only local SVG and bundled code; add no runtime network or diagram
  dependency.
- Preserve the user's existing `editors/vscode/package.json` version change.
- Honor VS Code themes, high contrast, keyboard operation, narrow layouts, and
  reduced motion.

## File map

| File | Responsibility |
|---|---|
| `editors/vscode/src/cli/processManager.ts` | Per-command UTF-8 output ceilings |
| `crates/compass-output/src/callflow_model.rs` | Complete `/2` call-flow model and scope classification |
| `crates/compass-output/tests/callflow_model.rs` | Model completeness and classification regression coverage |
| `packages/compass-viewer/src/contracts/callflow.ts` | Full CLI `/2` runtime schema |
| `packages/compass-viewer/src/contracts/architecture.ts` | Bounded host/webview projection types |
| `editors/vscode/src/views/architectureIndex.ts` | Host-side indexes, search, filters, paging, and coverage |
| `editors/vscode/src/transport/architectureMessages.ts` | Validated architecture message protocol |
| `editors/vscode/src/views/architecturePanel.ts` | Panel lifecycle, export, retained model, request routing |
| `editors/vscode/src/webviews/architecture.tsx` | Message bridge and controlled viewer state |
| `packages/compass-viewer/src/architecture/layout.ts` | Deterministic layered subsystem SVG layout |
| `packages/compass-viewer/src/architecture/ArchitectureMap.tsx` | Accessible directional system map |
| `packages/compass-viewer/src/architecture/ArchitectureFlow.tsx` | Three-pane workspace and inspector |
| `packages/compass-viewer/src/theme.css` | Architecture-specific responsive visual system |

---

### Task 1: Add architecture-specific process capacity

**Context:** The current `bounded()` helper uses JavaScript character length and
one global 8 MiB limit. Django emits 35,110,985 bytes, so architecture needs a
larger stdout ceiling without changing any other workflow.

**Files:**

- Modify: `editors/vscode/src/cli/processManager.ts`
- Modify: `editors/vscode/src/cli/processManager.test.ts`

**Interfaces:**

```ts
export type OutputLimits = {
  stdoutBytes?: number;
  stderrBytes?: number;
};

runJson<T>(
  cwd: string,
  args: readonly string[],
  schema: ZodType<T>,
  signal?: AbortSignal,
  limits?: OutputLimits
): Promise<T>;

run(
  cwd: string,
  args: readonly string[],
  signal?: AbortSignal,
  limits?: OutputLimits
): Promise<CommandResult>;

startCommand(
  cwd: string,
  args: readonly string[],
  limits?: OutputLimits
): RunningCommand;
```

- [ ] Replace string-length bounding with a small accumulator that tracks
      `Buffer.byteLength(chunk, "utf8")`, labels stdout versus stderr, kills the
      child on overflow, and defaults each stream to `8 * 1024 * 1024`.
- [ ] Thread optional limits through `run`, `startCommand`, `collect`, and
      `runJson` without changing existing call behavior.
- [ ] Add regression tests showing ordinary output rejects above 8 MiB,
      multibyte UTF-8 is measured by bytes, architecture-style output between
      8 and 128 MiB succeeds, and stderr remains capped at 8 MiB.
- [ ] Run:

```bash
npm test -w editors/vscode -- --run src/cli/processManager.test.ts
npm run typecheck -w editors/vscode
```

- [ ] Commit only Task 1 files:

```bash
git add editors/vscode/src/cli/processManager.ts \
  editors/vscode/src/cli/processManager.test.ts
git commit -m "fix(vscode): support bounded large architecture exports"
```

### Task 2: Emit a complete call-flow v2 model

**Context:** Version 1 drops detailed cross-section calls. Version 2 must make
the completeness invariant explicit and provide source scopes for the
production-first presentation.

**Files:**

- Modify: `crates/compass-output/src/callflow_model.rs`
- Modify: `crates/compass-output/src/lib.rs`
- Modify: `crates/compass-cli/src/capability_commands.rs`
- Modify: `editors/vscode/src/cli/compatibility.ts`
- Modify: `editors/vscode/src/cli/compatibility.test.ts`
- Modify: `packages/compass-viewer/src/contracts/callflow.ts`
- Create: `crates/compass-output/tests/callflow_model.rs`
- Modify: `crates/compass-cli/tests/viewer_export_cli.rs`

**Interfaces:**

```rust
pub const CALLFLOW_VIEWER_SCHEMA: &str = "compass.viewer.callflow/1";

pub enum CallflowSourceScope {
    Production,
    Test,
    Generated,
    Vendor,
    Unknown,
}

pub struct CallflowCrossSectionCall {
    pub source: String,
    pub target: String,
    pub source_section: String,
    pub target_section: String,
    pub relation: String,
    pub confidence: String,
}

pub struct CallflowCoverage {
    pub internal: usize,
    pub cross_section: usize,
    pub unassigned: usize,
}
```

`CallflowViewNode` adds `scope`. `CallflowViewSection` adds `node_count` and
`internal_call_count`. `CallflowViewModel` adds `cross_section_calls` and
`coverage`.

- [ ] Implement deterministic source-scope classification using normalized path
      segments. Recognize test directories/names, generated/build outputs,
      vendor/third-party directories, production paths, and missing paths in
      that precedence order.
- [ ] While assigning nodes to sections, build the endpoint-to-section map once.
      Classify every graph edge as internal, cross-section, or unassigned;
      retain complete cross-section records and assert totals through exported
      coverage.
- [ ] Update public exports, CLI capability negotiation, the extension
      requirement, Zod schemas, and CLI integration expectations from `/1` to
      `/2`.
- [ ] Add Rust regression coverage for all source scopes, detailed
      cross-section evidence, section counts, and:

```rust
assert_eq!(
    model.coverage.internal
        + model.coverage.cross_section
        + model.coverage.unassigned,
    document.links.len()
);
```

- [ ] Run:

```bash
cargo fmt --all
cargo test -p compass-output --test callflow_model
cargo test -p compass-cli --test viewer_export_cli callflow_json
npm run typecheck -w @compass/viewer
npm test -w editors/vscode -- --run src/cli/compatibility.test.ts
```

- [ ] Commit Task 2 files:

```bash
git add crates/compass-output crates/compass-cli/src/capability_commands.rs \
  crates/compass-cli/tests/viewer_export_cli.rs \
  editors/vscode/src/cli/compatibility.ts \
  editors/vscode/src/cli/compatibility.test.ts \
  packages/compass-viewer/src/contracts/callflow.ts
git commit -m "feat(callflow): preserve complete architecture evidence"
```

### Task 3: Build bounded host-side architecture projections

**Context:** The extension may retain the complete `/2` model, but the webview
must receive only the overview and requested pages. Projection code stays pure
and independent of VS Code so its filtering and completeness can be reviewed
directly.

**Files:**

- Create: `packages/compass-viewer/src/contracts/architecture.ts`
- Modify: `packages/compass-viewer/src/index.ts`
- Create: `editors/vscode/src/views/architectureIndex.ts`
- Create: `editors/vscode/src/views/architectureIndex.test.ts`
- Create: `editors/vscode/src/transport/architectureMessages.ts`
- Create: `editors/vscode/src/transport/architectureMessages.test.ts`

**Interfaces:**

```ts
export type ArchitectureScope = "production" | "all";
export type EvidenceFilter = "all" | "extracted" | "inferred" | "ambiguous";

export class ArchitectureIndex {
  constructor(model: CallflowViewModel);
  overview(scope: ArchitectureScope, evidence: EvidenceFilter): ArchitectureOverview;
  sectionPage(request: SectionPageRequest): ArchitectureSectionPage;
  routePage(request: RoutePageRequest): ArchitectureRoutePage;
  search(request: ArchitectureSearchRequest): ArchitectureSearchPage;
}
```

All page requests include `repositoryId`, `generation`, `requestId`, scope,
evidence, page, and page size. All responses echo those identities and include
`total`, `start`, and `end`.

- [ ] Define Zod schemas for overview summaries, connections, coverage, section
      pages, route pages, search pages, loading phases, errors, and webview
      requests.
- [ ] Implement normalized indexes for node labels, source paths, relations,
      section names, scopes, and evidence. Derive overview counts and routes
      from filtered internal and cross-section records.
- [ ] Enforce page sizes from 1 through 100 and cap search results at 100. Never
      include `CallflowViewModel` in a host-to-webview schema.
- [ ] Add regression tests for production defaults, all-code restoration,
      evidence filters, cross-route paging, complete-model search outside the
      current page, identity echoing, and invalid page-size rejection.
- [ ] Run:

```bash
npm test -w editors/vscode -- --run \
  src/views/architectureIndex.test.ts \
  src/transport/architectureMessages.test.ts
npm run typecheck -w editors/vscode
npm run typecheck -w @compass/viewer
```

- [ ] Commit Task 3 files:

```bash
git add packages/compass-viewer/src/contracts/architecture.ts \
  packages/compass-viewer/src/index.ts \
  editors/vscode/src/views/architectureIndex.ts \
  editors/vscode/src/views/architectureIndex.test.ts \
  editors/vscode/src/transport/architectureMessages.ts \
  editors/vscode/src/transport/architectureMessages.test.ts
git commit -m "feat(vscode): index architecture data in the extension host"
```

### Task 4: Convert the architecture panel to a paged controller

**Context:** `architecturePanel.ts` currently exports one function, accepts
unvalidated messages, and posts the full model. Convert it into a lifecycle
controller that owns the retained index and rejects stale work.

**Files:**

- Modify: `editors/vscode/src/views/architecturePanel.ts`
- Create: `editors/vscode/src/views/architecturePanel.test.ts`
- Modify: `editors/vscode/src/webviews/architecture.tsx`

**Interfaces:**

```ts
const ARCHITECTURE_STDOUT_LIMIT = 128 * 1024 * 1024;

class ArchitecturePanelController {
  hydrate(): Promise<void>;
  handleMessage(message: ArchitectureToHostMessage): Promise<void>;
  dispose(): void;
}
```

- [ ] Hydrate with `run(..., signal, { stdoutBytes:
      ARCHITECTURE_STDOUT_LIMIT })`, record `Buffer.byteLength(result.stdout,
      "utf8")`, parse with `CallflowViewModelSchema`, retain an
      `ArchitectureIndex`, and post only `architectureOverview`.
- [ ] Route validated section, route, search, scope, source, output, retry, and
      ready messages. Reject wrong repository identities and ignore stale
      generation/request responses.
- [ ] Update the webview adapter to maintain overview, detail-page, search,
      selection, and loading state. It sends typed requests and never receives
      the full `/2` model.
- [ ] Show explicit `exporting`, `validating`, `indexing`, and `mapping` loading
      copy. Report an actionable 128 MiB error without implying graph
      corruption.
- [ ] Add panel tests with fake process/webview boundaries proving 35 MiB
      hydration uses the 128 MiB option, only bounded overview data is posted,
      route requests page correctly, retry invalidates old work, and disposal
      cancels work and releases the index.
- [ ] Run:

```bash
npm test -w editors/vscode -- --run src/views/architecturePanel.test.ts
npm run typecheck -w editors/vscode
npm run build -w editors/vscode
```

- [ ] Commit Task 4 files:

```bash
git add editors/vscode/src/views/architecturePanel.ts \
  editors/vscode/src/views/architecturePanel.test.ts \
  editors/vscode/src/webviews/architecture.tsx
git commit -m "feat(vscode): page architecture data through the host"
```

### Task 5: Build the interactive system map and inspector

**Context:** Replace the flat relationship-card overview with a directional
three-pane workspace. Keep tables as exhaustive alternatives and make selection
state understandable without relying on color.

**Files:**

- Create: `packages/compass-viewer/src/architecture/layout.ts`
- Create: `packages/compass-viewer/src/architecture/layout.test.ts`
- Create: `packages/compass-viewer/src/architecture/ArchitectureMap.tsx`
- Create: `packages/compass-viewer/src/architecture/ArchitectureMap.test.tsx`
- Modify: `packages/compass-viewer/src/architecture/ArchitectureFlow.tsx`
- Modify: `packages/compass-viewer/src/architecture/state.ts`
- Modify: `packages/compass-viewer/src/architecture/state.test.ts`
- Create: `packages/compass-viewer/src/architecture/ArchitectureFlow.test.tsx`
- Modify: `packages/compass-viewer/src/theme.css`

**Interfaces:**

```ts
export function layoutArchitecture(
  sections: readonly ArchitectureSectionSummary[],
  routes: readonly ArchitectureRouteSummary[],
  viewport: { width: number; height: number }
): ArchitectureLayout;
```

`ArchitectureFlow` receives the current `ArchitectureOverview`, optional
section/route/search pages, loading state, and callbacks for scope, evidence,
selection, paging, search, and source opening.

- [ ] Implement a deterministic layered layout with stable section ordering,
      cycle-safe columns, logarithmically capped node area and route width, and
      coordinates suitable for fit/reset/pan.
- [ ] Render an SVG with titled nodes, directional curves, arrow markers,
      visible extracted/inferred line styles, keyboard-selectable nodes/routes,
      screen-reader labels, and a table alternative.
- [ ] Rebuild `ArchitectureFlow` as:
      searchable grouped subsystem rail; top scope/evidence/coverage toolbar;
      central map; right selection inspector; paged symbols/internal calls or
      cross-route evidence.
- [ ] Make `Production · X of Y symbols` visible on initial load. Keep test,
      generated, vendor, and unknown totals visible and expose them through
      `All code`.
- [ ] Replace the old card-grid CSS with restrained blueprint-like route
      styling derived from VS Code variables. Add high-contrast, reduced-motion,
      focus, and sub-760px inspector-below-map rules.
- [ ] Add regression tests for deterministic layout, directed accessible
      routes, incoming/outgoing highlighting, production disclosure, all-code
      scope, route evidence, table fallback, pagination, empty states, and
      source callbacks.
- [ ] Run:

```bash
npm test -w @compass/viewer -- --run src/architecture
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
npm run build -w editors/vscode
```

- [ ] Commit Task 5 files:

```bash
git add packages/compass-viewer/src/architecture \
  packages/compass-viewer/src/theme.css
git commit -m "feat(viewer): visualize complete architecture flow"
```

### Task 6: Qualify the Django flow and package the extension

**Context:** Unit tests protect contracts, but completion requires the original
35,110,985-byte reproduction plus the release-shaped extension bundle.

**Files:**

- Modify: `editors/vscode/CHANGELOG.md`
- Refresh: parent repository `graphify-out/`

- [ ] Add a changelog entry describing the production-first system map,
      complete cross-subsystem evidence, and 128 MiB architecture-only export
      capacity.
- [ ] Run the complete focused suites:

```bash
cargo test -p compass-output
cargo test -p compass-cli --test viewer_export_cli
npm test -w @compass/viewer -- --run src/architecture
npm test -w editors/vscode
npm run typecheck:js
npm run build:vscode
```

- [ ] Measure the real Django export and inspect the `/2` completeness invariant:

```bash
set -o pipefail
cargo run -p compass-cli -- export callflow-json \
  --graph /Users/haipingfu/Github/django/compass-out/graph.json |
  jq '{
    schema,
    bytes: (tostring | utf8bytelength),
    nodes: .statistics.nodes,
    edges: .statistics.edges,
    classifiedEdges:
      (.coverage.internal + .coverage.crossSection + .coverage.unassigned)
  }'
```

Expected: schema `/2`, bytes below 134,217,728, 50,944 nodes, 190,401 edges,
and `classifiedEdges == edges`.

- [ ] Launch the Extension Development Host against Django and verify:
      production scope is disclosed; map selection highlights both directions;
      `All code` exposes tests/generated code; a cross-subsystem route pages to
      its final call; source navigation opens a real file; retry and output
      actions work.
- [ ] Build and inspect the release artifact:

```bash
npm run package -w editors/vscode
npm run smoke:vsix -w editors/vscode
```

- [ ] Refresh the parent graph after all code changes:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

- [ ] Review `git diff --check`, `git status --short`, the parent/submodule
      boundary, and preserve the pre-existing package-version edit. Commit the
      changelog entry:

```bash
git add editors/vscode/CHANGELOG.md
git commit -m "docs(vscode): document scalable architecture flow"
```

## Completion checklist

- [ ] Django loads without the 8 MiB architecture failure.
- [ ] Ordinary commands still reject stdout or stderr above 8 MiB.
- [ ] The webview never receives the complete retained model.
- [ ] Production scope and full totals are both visible.
- [ ] All-code search reaches tests, generated, vendor, and unknown sources.
- [ ] Internal, cross-section, and unassigned coverage equals total edges.
- [ ] Cross-subsystem routes expose complete paged call evidence.
- [ ] SVG, table fallback, keyboard use, high contrast, reduced motion, and
      narrow layouts preserve the same information.
- [ ] Rust, TypeScript, viewer, extension, build, package, VSIX smoke, Django,
      and graph-refresh checks have fresh passing evidence.
