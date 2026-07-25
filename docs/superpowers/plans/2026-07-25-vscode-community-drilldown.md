# VS Code Community Drill-Down Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add lazy, Compass-powered community drill-down to current and historical VS Code graphs while making `json` the canonical graph-view export name.

**Architecture:** Compass filters the original graph into a validated `compass.viewer.graph/1` community-detail model. VS Code requests details through typed host messages, keeps the overview in memory, and conditionally routes community versus source double-clicks. The active standalone HTML exporter is unchanged.

**Tech Stack:** Rust, serde, Compass graph/history crates, TypeScript, Zod, React, vis-network, VS Code webviews, Vitest, Playwright.

## Global Constraints

- Use only the installed Compass CLI for extension workflows; never invoke Graphify.
- `compass export json` is canonical; `viewer-json` remains a deprecated compatibility alias.
- Support current and historical graphs.
- Implement behavior before adding regression tests; do not use a TDD red-green loop.
- Preserve `compass.viewer.graph/1` compatibility and the active `compass export html` renderer.
- Community responses are complete or fail; never silently truncate them.
- Double-click community enters detail, double-click located source opens source, Back restores overview.

---

### Task 1: Compass Community View Model and Canonical JSON Export

**Files:**
- Modify: `crates/compass-output/src/html.rs`
- Modify: `crates/compass-output/src/lib.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/src/capability_commands.rs`
- Test after implementation: `crates/compass-output/src/viewer_model.rs`
- Test after implementation: `crates/compass-cli/tests/viewer_export_cli.rs`
- Test after implementation: `crates/compass-cli/tests/capabilities_cli.rs`

**Interfaces:**
- Produces: `graph_community_view_model_document(document, communities, title, options, community_id) -> Result<GraphViewModel, OutputError>`
- Produces: canonical `compass export json --community ID`
- Produces: compatibility alias `compass export viewer-json --community ID`
- Produces: capability feature `community_detail: true`

- [ ] **Step 1: Add explicit community errors and filtering**

Add `OutputError::UnknownCommunity { community: usize }` and
`OutputError::CommunityTooLarge { community, nodes, limit }`. Implement a helper
that resolves `communities[community_id]`, enforces the positive node limit,
selects member nodes, retains links with both endpoints selected, and filters
hyperedges so no emitted member is outside the selection.

- [ ] **Step 2: Produce the detail view model**

Call `graph_view_model` on the filtered document with the selected community
map and `aggregated = false`. Preserve labels, learning metadata, node colors,
source metadata, and the existing schema.

- [ ] **Step 3: Make `json` canonical in the current export parser**

Accept both `"json"` and `"viewer-json"`, parse `--community` as a non-negative
integer only for those formats, and route both aliases through one helper:

```rust
let model = if let Some(community) = community {
    graph_community_view_model_document(
        &inputs.document,
        &inputs.communities,
        &graph_path,
        &options,
        community,
    )
    .map_err(|error| error.to_string())
} else {
    graph_view_model_document(
        &inputs.document,
        &inputs.communities,
        &graph_path,
        &options,
    )
    .map_err(|error| error.to_string())
    .and_then(|model| {
        model.ok_or_else(|| "graph has no renderable community overview".to_owned())
    })
};
```

Return a specific “graph has no renderable community overview” error for the
existing `None` path rather than reusing the community error.

- [ ] **Step 4: Update public help and capability negotiation**

Primary help and examples show `compass export json`; format-specific help
documents `--community ID`. Keep `viewer-json` accepted but omit it from the
primary command list. Add `("community_detail", true)` to capabilities without
changing `graph_viewer`.

- [ ] **Step 5: Add post-implementation Rust and CLI coverage**

Cover exact member filtering, internal edges, safe hyperedges, unknown
community, oversized community, canonical `json`, alias compatibility, and the
capability flag.

- [ ] **Step 6: Run focused checks and commit**

Run:

```text
cargo fmt --all -- --check
cargo test -p compass-output viewer_model --locked
cargo test -p compass-cli --test viewer_export_cli --test capabilities_cli --locked
```

Commit:

```text
feat(cli): add lazy community JSON export
```

---

### Task 2: Historical Community Export

**Files:**
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Test after implementation: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Consumes: `graph_community_view_model_document`
- Produces: `compass history export REV --format json --community ID --output PATH`
- Preserves: deprecated `--format viewer-json`

- [ ] **Step 1: Parse historical community selection**

Extend the existing history option parser with `community: Option<usize>`.
Reject `--community` for `graph-json` and `compass-out`. Treat `json` and
`viewer-json` as the same graph presentation branch.

- [ ] **Step 2: Export the selected historical detail**

After validating the preferred realization, use its exact document,
communities, labels, commit, realization, and fingerprint. Place the detail
model in the existing `compass.history.viewer_graph/1` envelope so the
`RevisionStore` validation remains unchanged.

- [ ] **Step 3: Update history help**

Show:

```text
history export REV --format json [--community ID] --output PATH
```

Document `viewer-json` only in compatibility notes.

- [ ] **Step 4: Add post-implementation history tests**

Materialize a two-community revision, export one community, assert the envelope
identity and internal graph, then cover unknown IDs, oversized details, and the
deprecated alias.

- [ ] **Step 5: Run and commit**

Run:

```text
cargo test -p compass-cli --test history_cli --locked
```

Commit:

```text
feat(history): export lazy community details
```

---

### Task 3: VS Code Host Transport and Detail Loading

**Files:**
- Modify: `packages/compass-viewer/src/contracts/graph.ts`
- Modify: `editors/vscode/src/transport/messages.ts`
- Modify: `editors/vscode/src/views/graphPanel.ts`
- Modify: `editors/vscode/src/history/revisionStore.ts`
- Modify: `editors/vscode/src/views/historyPanel.ts`
- Modify: `editors/vscode/src/webviews/graph.tsx`
- Modify: `editors/vscode/src/webviews/history.tsx`
- Create after implementation: `editors/vscode/src/views/communityArguments.test.ts`

**Interfaces:**
- Produces: webview request `{ type: "openCommunity", repositoryId, communityId }`
- Produces: host response `{ type: "communityGraph", requestId, communityId, graph }`
- Produces: host failure `{ type: "communityError", requestId, communityId, message }`
- Produces: `RevisionStore.loadCommunity(commit, communityId, nodeLimit)`

- [ ] **Step 1: Add typed graph messages**

Use Zod non-negative integer validation for `communityId`. Every response
includes a random request ID so stale results can be discarded.

- [ ] **Step 2: Add current-graph loading**

`GraphPanel` retains the hydrated overview, verifies the community ID exists
and has `memberCount`, then runs:

```ts
["export", "json", "--graph", session.graphPath, "--node-limit", String(limit),
 "--community", String(communityId)]
```

Validate with `GraphViewModelSchema`, cache at most three details, and clear the
cache when the panel rehydrates.

- [ ] **Step 3: Add historical loading**

Change ordinary revision loading to canonical `--format json`. Add
`RevisionStore.loadCommunity` using an unpredictable temporary file, the exact
revision, `--format json`, `--community`, validation of the historical
envelope, and guaranteed cleanup.

- [ ] **Step 4: Connect history identities**

Track the commit whose graph is currently displayed. Reject community requests
for another commit and discard responses if the selected graph changes before
the export returns.

- [ ] **Step 5: Add post-implementation transport tests**

Assert exact Compass argument arrays, invalid IDs, repository mismatches,
three-entry cache eviction, stale-response rejection, and temporary cleanup.

- [ ] **Step 6: Run and commit**

Run:

```text
npm run typecheck -w compass-vscode
npm run test -w compass-vscode
```

Commit:

```text
feat(vscode): load community graphs through Compass
```

---

### Task 4: React Drill-Down, Back Navigation, and Conditional Double-Click

**Files:**
- Modify: `packages/compass-viewer/src/graph/CompassGraph.tsx`
- Modify: `packages/compass-viewer/src/graph/networkEvents.ts`
- Modify: `packages/compass-viewer/src/graph/GraphToolbar.tsx`
- Modify: `packages/compass-viewer/src/graph/GraphInspector.tsx`
- Modify: `packages/compass-viewer/src/graph/state.ts`
- Modify: `packages/compass-viewer/src/theme.css`
- Modify: `packages/compass-viewer/src/history/HistoryWorkspace.tsx`
- Test after implementation: `packages/compass-viewer/src/graph/state.test.ts`
- Test after implementation: `packages/compass-viewer/src/graph/networkEvents.test.ts`
- Test after implementation: `tests/viewer/fixtures/generate.ts`
- Test after implementation: `tests/viewer/graph-parity.spec.ts`
- Test after implementation: `tests/viewer/history.spec.ts`

**Interfaces:**
- Extends `GraphHost` with `openCommunity(communityId): void`
- Accepts optional `communityDetail`, `communityLoading`, `communityError`,
  and `onBackToOverview`

- [ ] **Step 1: Add overview/detail state**

Store active detail identity separately from graph interaction state. Reset
focus, hover, hidden communities, query, and canvas view whenever the displayed
model changes.

- [ ] **Step 2: Route double-click conditionally**

Replace the source-only handler with:

```ts
if (model.stats.aggregated && node.memberCount !== undefined) {
  host.openCommunity(node.community);
  return;
}
const source = navigableSource(node);
if (source) host.openSource(source);
```

- [ ] **Step 3: Add loading, failure, and Back UI**

Keep the overview mounted during loading, show an accessible toolbar/inspector
status, disable duplicate community requests, and show **Back to community
overview** only in detail mode. Failure leaves the overview active and retryable.

- [ ] **Step 4: Connect current and historical webviews**

Graph and history entry points retain the overview, accept typed community
responses, ignore stale request IDs, and pass the active detail to
`CompassGraph`.

- [ ] **Step 5: Add post-implementation React and Chromium coverage**

Build an aggregated fixture with a lazy community response. Prove actual canvas
double-click requests the community, success enters member nodes, source
double-click still opens the exact range, failure preserves overview, and Back
restores overview. Repeat the request/Back flow for a historical graph.

- [ ] **Step 6: Run and commit**

Run:

```text
npm run typecheck:js
npm run test:js
```

Commit:

```text
feat(viewer): navigate lazy community details
```

---

### Task 5: Documentation, Compass Refresh, and Release Qualification

**Files:**
- Modify: `docs/reference/commands.md`
- Modify: `docs/guides/vscode.md`
- Modify: `editors/vscode/README.md`
- Modify: `editors/vscode/CHANGELOG.md`
- Regenerate: `crates/compass-output/assets/viewer/graph.js`
- Regenerate: `crates/compass-output/assets/viewer/viewer.css`
- Regenerate: `crates/compass-output/assets/viewer/manifest.json`

**Interfaces:**
- Documents canonical `json`, compatibility alias, lazy loading, conditional
  double-click, Back navigation, limits, and upgrade guidance.

- [ ] **Step 1: Update documentation and examples**

Replace new `viewer-json` examples with `json`. State that `viewer-json` remains
a deprecated alias. Document current and historical `--community`.

- [ ] **Step 2: Regenerate deterministic viewer assets**

Run:

```text
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
```

- [ ] **Step 3: Refresh with Compass**

Run:

```text
cargo build -p compass-cli --bin compass
target/debug/compass capabilities --format json
target/debug/compass update . --no-viz
```

Use `--force` only if Compass’s shrink guard explicitly requires it. Never run
`graphify update`.

- [ ] **Step 4: Run release qualification**

Run:

```text
cargo fmt --all -- --check
cargo clippy -p compass-cli -p compass-history -p compass-output --all-targets --locked -- -D warnings
cargo test --workspace --exclude compass-parity --lib --bins --locked
cargo test -p compass-cli --test capabilities_cli --test viewer_export_cli --test history_cli --locked
npm run typecheck:js
npm run test:js
npm run test:integration -w compass-vscode
node scripts/check_viewer_assets.mjs
npm audit --omit=dev
npm run package -w compass-vscode
npm run smoke:vsix -w compass-vscode
```

- [ ] **Step 5: Review, commit, push, and update PR**

Request a read-only code review, fix all Critical/Important findings, verify the
tracked tree is clean, push `feature/compass-vscode-extension`, and update PR
#33 with the new commands, behavior, tests, VSIX checksum, and Compass graph
statistics.

Commit:

```text
docs(vscode): document community drill-down
```
