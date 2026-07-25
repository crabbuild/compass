# Compass VS Code Evolution and Diff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show every Git commit with its Compass materialization state, explicitly build missing revisions, load exact historical graphs, and compare revisions with graph and semantic evidence.

**Architecture:** A new inspection-only timeline command joins Git topology, immutable history catalog state, and operational jobs without creating storage. The extension virtualizes that model, coordinates builds through existing history commands, exports exact revision graphs into private storage, and presents comparisons with the shared viewer.

**Tech Stack:** Rust, Git, Compass history/semantic-diff, React, TypeScript, Zod, VS Code Extension API, Vitest, browser tests

## Global Constraints

- Complete plans 01 and 02 first.
- Timeline inspection never enables history, creates a store/queue, or materializes a graph.
- Include every commit reachable from local refs plus retained materialized commits whose Git objects disappeared.
- Show available, missing, queued, building, failed, corrupt, and incompatible states explicitly.
- Missing commits build only after explicit user action.
- Historical graphs must match exact full SHA and preferred realization ID; never substitute.
- Keep at most three decoded revision graphs and bounded private export files.
- Preserve shared-node positions across revisions.
- Merge comparison requires explicit parent selection when necessary.
- Comparison is disabled for missing parent realization or incompatible profiles.
- Run `graphify update .` after code changes.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/compass-history/src/timeline.rs` | Inspection-only Git/history/job union and state classification. |
| `crates/compass-cli/src/history_commands.rs` | `history timeline --format json` and JSONL build progress. |
| `packages/compass-viewer/src/history/*` | Timeline, commit details, cache, revision graph, and comparison UI. |
| `editors/vscode/src/history/*` | Timeline client, build coordinator, private exports, cache, and cleanup. |
| `editors/vscode/src/views/historyPanel.ts` | History editor tab host and transport. |
| `editors/vscode/src/webviews/history.tsx` | React history entry. |

### Task 1: Add an inspection-only repository timeline model

**Files:**
- Create: `crates/compass-history/src/timeline.rs`
- Modify: `crates/compass-history/src/lib.rs`
- Modify: `crates/compass-history/src/git.rs`
- Create: `crates/compass-history/tests/timeline.rs`

**Interfaces:**
- Produces: `TimelineRequest`, `TimelineEntry`, `TimelineGraphState`, `CommitPresentation`, and `Repository::timeline`.
- Consumes: optional `&HistoryStore` and optional `&HistoryQueue` opened with non-creating APIs.

- [ ] **Step 1: Write non-mutation and all-commit tests**

```rust
#[test]
fn timeline_lists_all_refs_without_creating_history() -> Result<(), Box<dyn Error>> {
    let fixture = divergent_branch_fixture()?;
    let repository = Repository::discover(fixture.path())?;
    let before = fixture.snapshot_paths();
    let entries = repository.timeline(None, None, &TimelineRequest::default())?;
    assert_eq!(entries.iter().map(|e| e.commit.as_str()).collect::<BTreeSet<_>>(), fixture.all_commits());
    assert!(entries.iter().all(|entry| entry.graph_state == TimelineGraphState::Missing));
    assert_eq!(fixture.snapshot_paths(), before);
    Ok(())
}
```

- [ ] **Step 2: Implement NUL-safe Git topology enumeration**

Invoke Git with `log --all --topo-order --date-order` and format full SHA, parents, author, ISO timestamp, and subject as NUL-delimited fields. Reject malformed/non-UTF-8 output. Add retained `HistoryStore::list(None)` commit IDs absent from Git with unavailable presentation and stored parents.

- [ ] **Step 3: Classify graph and job state**

```rust
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TimelineGraphState {
    Available { realization: RealizationId, fingerprint: String },
    Missing,
    Queued { job_id: String },
    Building { job_id: String },
    Failed { job_id: String, diagnostic: String },
    Corrupt { realization: RealizationId, diagnostic: String },
    Incompatible { realization: RealizationId, schema: u32 },
}
```

Prefer a current non-terminal job over missing; prefer a validated preferred realization over old terminal jobs; report validation/catalog failure as corrupt rather than dropping the commit.

- [ ] **Step 4: Verify no filesystem mutation and commit**

Run:

```bash
cargo fmt --all
cargo test -p compass-history --test timeline
graphify update .
git add crates/compass-history
git commit -m "feat(history): expose inspection-only repository timeline"
```

### Task 2: Expose timeline and history build progress through CLI contracts

**Files:**
- Modify: `crates/compass-cli/src/history_commands.rs`
- Modify: `crates/compass-cli/src/ide_contract.rs`
- Modify: `crates/compass-cli/src/capability_commands.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Produces: `compass history timeline [--rev REV] --format json` schema `compass.history.timeline/1`, `compass history change-counts REV [--parent REV] --format json` schema `compass.history.change_counts/1`, `compass history export REV --format viewer-json --output PATH` schema `compass.history.viewer_graph/1`, and JSONL events for `history build`.
- Consumes: `Repository::timeline`.

- [ ] **Step 1: Add the failing inspection-only CLI test**

```rust
let before = snapshot_repository(&root)?;
let output = run(&root, ["history", "timeline", "--format", "json"])?;
assert_eq!(output.code, 0, "{}", output.stderr);
let value: Value = serde_json::from_str(&output.stdout)?;
assert_eq!(value["schema"], "compass.history.timeline/1");
assert_eq!(value["entries"].as_array().map(Vec::len), Some(3));
assert_eq!(snapshot_repository(&root)?, before);
```

- [ ] **Step 2: Implement timeline command**

Default to all local refs. `--rev REV` limits to commits reachable from the resolved revision but still includes retained realizations directly attached to those commits. JSON includes repository ID, selected HEAD, entries, parents, presentation availability, graph state, and configured history-enabled state.

- [ ] **Step 3: Add build JSONL events**

Map enqueue, claim, materialize stages, validation, publication, retry, and terminal outcome to `compass.ide.progress/1`. For `--all`, include commit index/total and continue-after-failure summary. Human output remains unchanged without `--events jsonl`.

- [ ] **Step 4: Add exact viewer export and lazy structural counts**

`history export REV --format viewer-json --output PATH` validates the preferred
realization, reconstructs its graph, calls `graph_view_model`, and atomically
writes:

```rust
#[derive(Serialize)]
struct HistoricalGraphView {
    schema: &'static str, // "compass.history.viewer_graph/1"
    commit: CommitId,
    realization: RealizationId,
    fingerprint: ExtractionFingerprint,
    graph: GraphViewModel,
}
```

`history change-counts` resolves the selected parent (first parent by default),
requires existing comparable preferred realizations, streams `HistoryStore::diff`
into node/edge/hyperedge added/removed/changed counters, and never
materializes. Timeline clients request these counts lazily for selected or
visible commits rather than diffing every commit during initial load.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo fmt --all
cargo test -p compass-cli --test history_cli --test capabilities_cli
graphify update .
git add crates/compass-cli
git commit -m "feat(cli): expose history timeline and build events"
```

### Task 3: Build the virtualized history timeline and state reducer

**Files:**
- Create: `packages/compass-viewer/src/contracts/history.ts`
- Create: `packages/compass-viewer/src/history/state.ts`
- Create: `packages/compass-viewer/src/history/state.test.ts`
- Create: `packages/compass-viewer/src/history/HistoryWorkspace.tsx`
- Create: `packages/compass-viewer/src/history/HistoryWorkspace.test.tsx`
- Create: `packages/compass-viewer/src/history/CommitRail.tsx`
- Create: `packages/compass-viewer/src/history/CommitDetails.tsx`
- Modify: `packages/compass-viewer/src/index.ts`

**Interfaces:**
- Consumes: `compass.history.timeline/1`.
- Produces: `HistoryWorkspace`, `historyReducer`, `HistoryHost`, and virtualized `CommitRail`.

- [ ] **Step 1: Test state preservation and explicit build behavior**

```ts
it("selecting a missing commit requests no build until the user acts", async () => {
  const host = { loadRevision: vi.fn(), buildRevision: vi.fn() };
  render(<HistoryWorkspace timeline={fixtureTimeline} host={host} />);
  await userEvent.click(screen.getByRole("option", { name: /missing commit/i }));
  expect(host.loadRevision).not.toHaveBeenCalled();
  expect(host.buildRevision).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: /build graph/i }));
  expect(host.buildRevision).toHaveBeenCalledWith(missingSha);
});
```

- [ ] **Step 2: Implement timeline runtime schema and reducer**

Track full SHA selection, filters, visible lanes, graph load state, per-commit operation state, selected parent, comparison state, and non-blocking messages. URL fragments are not used in VS Code; serialize full SHA through `WebviewPanelSerializer`.

- [ ] **Step 3: Implement a virtualized lane rail**

Render only visible rows plus overscan while maintaining parent-lane columns. Every commit remains keyboard-selectable through listbox semantics and searchable by SHA, subject, author, state, and date. State uses icon, label, and color.

- [ ] **Step 4: Verify and commit**

Run:

```bash
npm test -w @compass/viewer -- --run src/history
npm run typecheck:js
graphify update .
git add packages/compass-viewer
git commit -m "feat(viewer): add complete Git evolution timeline"
```

### Task 4: Add exact historical graph export, private caching, and cleanup

**Files:**
- Create: `editors/vscode/src/history/timelineClient.ts`
- Create: `editors/vscode/src/history/revisionStore.ts`
- Create: `editors/vscode/src/history/revisionStore.test.ts`
- Create: `editors/vscode/src/history/lru.ts`
- Create: `editors/vscode/src/history/lru.test.ts`
- Create: `editors/vscode/src/views/historyPanel.ts`
- Create: `editors/vscode/src/webviews/history.tsx`
- Modify: `editors/vscode/src/transport/messages.ts`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/package.json`

**Interfaces:**
- Consumes: history timeline and `history export REV --format viewer-json`.
- Produces: `RevisionStore.load(sha)`, three-entry `LruCache`, and History panel hydration.

- [ ] **Step 1: Test identity validation and eviction**

```ts
it("rejects a historical export whose commit identity differs", async () => {
  await expect(store.load(expectedSha, fakeExport({ commit: otherSha })))
    .rejects.toThrow(/expected revision/);
});

it("keeps at most three decoded revisions", () => {
  const cache = new LruCache<string, object>(3);
  ["a", "b", "c", "d"].forEach(key => cache.set(key, {}));
  expect(cache.keys()).toEqual(["d", "c", "b"]);
});
```

- [ ] **Step 2: Implement private staged exports**

Create per-repository storage below `ExtensionContext.storageUri` with owner-only permissions where supported. Use unpredictable filenames and `history export <sha> --format viewer-json --output <temp>`. Validate the envelope schema, full SHA, preferred realization, and fingerprint returned by the timeline before hydrating its nested graph model.

- [ ] **Step 3: Implement bounded lifecycle**

Keep three decoded models in memory, cap stored exports by count and total bytes, delete least-recently-used exports, and clean abandoned files on activation. Never delete outside the repository's allocated extension storage directory.

- [ ] **Step 4: Restore exact tab identity**

Serialize repository ID and full SHA. On restoration, refresh timeline, require that exact commit, and display a commit-local error if unavailable rather than selecting HEAD.

- [ ] **Step 5: Bind query and change counts to the selected revision**

Modify `editors/vscode/src/views/queryPanel.ts` so an available selected
historical commit is shown in the query header and adds `--at <full-sha>` to
natural-language and CompassQL arguments. Disable historical execution for
missing/failed/corrupt states; never let a query implicitly materialize a
missing timeline entry. Request `history change-counts <sha> --format json`
only for selected/visible comparable commits and cache the versioned response.

- [ ] **Step 6: Verify and commit**

Run:

```bash
npm test -w @compass/vscode -- --run src/history
npm run build -w @compass/vscode
npm run typecheck:js
graphify update .
git add editors/vscode
git commit -m "feat(vscode): load exact historical Compass graphs"
```

### Task 5: Add explicit historical build workflows

**Files:**
- Create: `editors/vscode/src/history/buildHistory.ts`
- Create: `editors/vscode/src/history/buildArguments.ts`
- Create: `editors/vscode/src/history/buildArguments.test.ts`
- Modify: `editors/vscode/src/views/historyPanel.ts`
- Modify: `editors/vscode/src/workspace/repositorySession.ts`

**Interfaces:**
- Consumes: `history enable|disable|build|rebuild|prefer|gc`, JSONL progress.
- Produces: guided history actions and live timeline state.

- [ ] **Step 1: Test profile-safe argument construction**

```ts
expect(buildHistoryArgs({
  sha, profile: { kind: "from", source: parentSha }, rebuild: false
})).toEqual([
  "history", "build", sha, "--profile-from", parentSha,
  "--format", "json", "--events", "jsonl"
]);
```

- [ ] **Step 2: Implement explicit forms**

Offer code-only, configured profile, or `--profile-from`; expose bulk/first-parent only from a separate confirmed action. Rebuild explains alternate realizations. Prefer and GC show realization IDs and require confirmation; GC passes `--yes` only after VS Code modal confirmation.

- [ ] **Step 3: Merge progress into the timeline**

Update queued/building/failed state from events, then refresh authoritative timeline after the terminal event. Cancellation leaves the final authoritative job state visible.

- [ ] **Step 4: Verify and commit**

Run:

```bash
npm test -w @compass/vscode -- --run src/history/buildArguments.test.ts
npm run typecheck:js
graphify update .
git add editors/vscode
git commit -m "feat(vscode): guide historical graph materialization"
```

### Task 6: Add parent graph comparison and semantic findings

**Files:**
- Create: `packages/compass-viewer/src/history/compare.ts`
- Create: `packages/compass-viewer/src/history/compare.test.ts`
- Create: `packages/compass-viewer/src/history/ComparisonOverlay.tsx`
- Create: `packages/compass-viewer/src/history/SemanticFindings.tsx`
- Create: `editors/vscode/src/history/diffClient.ts`
- Create: `editors/vscode/src/history/diffClient.test.ts`
- Modify: `editors/vscode/src/views/historyPanel.ts`
- Modify: `editors/vscode/src/transport/messages.ts`

**Interfaces:**
- Consumes: two `GraphViewModel`s and `compass diff OLD NEW --format json`.
- Produces: `computeGraphOverlay`, `DiffClient`, comparison overlay, and finding-to-evidence focus.

- [ ] **Step 1: Test stable-node layout transfer and graph changes**

```ts
const overlay = computeGraphOverlay(oldModel, newModel);
expect(overlay.nodes.get("shared")?.change).toBe("unchanged");
expect(overlay.nodes.get("removed")?.change).toBe("removed");
expect(overlay.nodes.get("added")?.change).toBe("added");
expect(transferPositions(oldPositions, newModel.nodes).get("shared")).toEqual({ x: 4, y: 8 });
```

- [ ] **Step 2: Implement deterministic overlay identity**

Compare nodes by opaque ID and edges by endpoint, relation, and occurrence-safe edge ID. Preserve changed attributes as old/new records. Keep removed nodes inspectable but visually historical.

- [ ] **Step 3: Implement semantic diff client**

Run `compass diff <parent> <selected> --format json`, validate `compass.semantic_diff.report/1`, and map findings, affected consumers, witness paths, and test evidence. Treat profile incompatibility as a disabled comparison with the CLI's rebuild guidance.

- [ ] **Step 4: Implement merge-parent selection and finding focus**

Default ordinary commits to their sole/first parent. For merges, require a parent picker before loading. Selecting a finding filters/focuses its node IDs and opens source evidence through the existing host service.

- [ ] **Step 5: Run the evolution milestone gate and commit**

Run:

```bash
cargo test -p compass-history -p compass-cli
npm test -w @compass/viewer -- --run src/history
npm test -w @compass/vscode -- --run src/history
npm run typecheck:js
graphify update .
git add packages/compass-viewer editors/vscode
git commit -m "feat(vscode): compare codebase evolution with semantic evidence"
```

### Task 7: Rebuild the offline all-history export on the shared viewer

**Files:**
- Create: `crates/compass-output/src/history_html.rs`
- Modify: `crates/compass-output/src/lib.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Create: `crates/compass-output/tests/history_html.rs`
- Create: `packages/compass-viewer/src/history/export-entry.tsx`
- Modify: `scripts/build_viewer_assets.mjs`
- Modify: `docs/superpowers/plans/2026-07-23-versioned-history-html-export.md`

**Interfaces:**
- Consumes: shared history components and validated preferred realizations.
- Produces: self-contained `compass history export --output history.html` with the same timeline/graph behavior.

- [ ] **Step 1: Mark the older static-viewer plan as superseded for presentation**

Add a note that its storage, validation, atomic no-clobber, compression, and identity requirements remain authoritative, while its hand-written static UI tasks are replaced by this shared React viewer task. Do not execute two competing renderer implementations.

- [ ] **Step 2: Add failing shared-export tests**

```rust
assert!(html.contains("compass.history.timeline/1"));
assert!(html.contains("data-viewer-build="));
assert!(!html.contains("<script src="));
assert!(!html.contains("https://"));
assert_eq!(embedded_payload_count(&html)?, preferred_realization_count);
```

- [ ] **Step 3: Implement the offline export entry**

Use the shared `HistoryWorkspace` in offline mode. Replace host callbacks with embedded payload lookup, browser fragment navigation, and parent comparison from embedded data. Preserve independent payload digest verification, at-most-three decoded graphs, corrupt-commit isolation, strict CSP, no-clobber publication, and 256 MiB `--force` confirmation from the earlier approved history-export design.

- [ ] **Step 4: Verify and commit**

Run:

```bash
npm run build:viewer
cargo test -p compass-output --test history_html
cargo test -p compass-cli --test history_cli
npm test -w @compass/viewer -- --run src/history
graphify update .
git add crates/compass-output crates/compass-cli packages/compass-viewer scripts docs/superpowers/plans/2026-07-23-versioned-history-html-export.md
git commit -m "feat(history): share the versioned graph viewer"
```
