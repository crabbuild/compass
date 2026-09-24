# Plan 026: Make the derived community overview answer coupling questions

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `advisor-plans/README.md` unless a reviewer told you they maintain the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3fd246dc..HEAD -- \
>   packages/compass-viewer/src/graph/communityOverview.ts \
>   packages/compass-viewer/src/graph/renderingProfile.ts \
>   packages/compass-viewer/src/graph/VisNetworkCanvas.tsx \
>   packages/compass-viewer/src/graph/CompassGraph.tsx \
>   packages/compass-viewer/src/graph/GraphToolbar.tsx \
>   packages/compass-viewer/src/graph/GraphInspector.tsx \
>   packages/compass-viewer/src/graph/EdgeHoverCard.tsx \
>   packages/compass-viewer/src/graph/edgeLabels.ts \
>   packages/compass-viewer/src/theme.css \
>   tests/viewer
> ```
>
> If a community-overview, community-bubble, or scope-toggle change landed
> since this plan was written, reconcile it with the steps below and stop
> rather than building a second parallel mechanism. Plan 027 (hierarchy
> artifact) supersedes the label ranking here when it lands; do not implement
> the Rust side in this plan.
>
> **Baseline requirement**: this plan builds on the derived community overview
> that was written but not yet committed when the plan was authored. Before
> Step 1, confirm the baseline is present:
> `rg -n "communityOverviewApplies|communityBubbleLabel|compass-scope-toggle" packages/compass-viewer/src` must return matches.
> If it returns nothing, the baseline work is missing — STOP and report instead
> of rebuilding it from scratch. If the baseline is uncommitted, commit it (or
> have the operator commit it) before starting, so the drift check below is
> meaningful.

## Status

- **Priority**: P1 — highest reader-visible value per unit of work
- **Effort**: M (viewer-only; no Rust changes)
- **Risk**: LOW — presentation only; no machine contract, artifact, or query change
- **Depends on**: none
- **Coordinates with**: Plan 027 (this plan's local ranking becomes the fallback when an exported hierarchy exists)
- **Category**: direction / UX
- **Planned at**: commit `3fd246dc`, 2026-09-23

## Why this matters

Compass already derives a community overview for large unaggregated graphs, but
that overview answers only "how big are the communities". The two questions a
reader actually brings to a code graph are "which subsystems matter" and "what
binds these two subsystems together". Today the second question is unanswerable
by construction: the derived overview reduces every cross-community relationship
to `"<count> cross-community edges"`, so a coupling of 40 calls and a coupling
of 40 shared type references look identical.

This plan makes the derived overview state the relationship mix, rank its labels
by more than raw size, and give the reader a breadcrumb plus per-row open
actions. It is deliberately viewer-only: the same data (`compass.viewer.graph/1`)
already carries relationship kinds, so no artifact, schema, or compatibility
change is involved.

## Current state

- `packages/compass-viewer/src/graph/communityOverview.ts` (new module from the
  overview work) derives the overview. Its aggregation currently discards
  relationship kinds:

  ```ts
  // communityOverview.ts — aggregateCommunityEdges
  counts.set(key, { source: low, target: high, count: 1 });
  ...
  relation: `${count} cross-community edges`,
  weight: count,
  confidence: "aggregated" as const
  ```

  Exports that already carry a Rust-built overview
  (`stats.aggregated === true`) keep whatever the exporter wrote, which is the
  same `"<count> cross-community edges"` string produced by
  `crates/compass-output/src/html.rs` (`fn aggregate`, line ~1113).

- `packages/compass-viewer/src/graph/semanticAppearance.ts` already defines the
  category vocabulary this plan reuses:
  `NODE_SEMANTIC_CATEGORIES = ["callable","type","module","boundary","other"]`,
  `EDGE_SEMANTIC_CATEGORIES = ["execution","dependency","structure","flow","other"]`,
  `edgeSemanticCategory(relation)`, and the boundary-kind set (`route`,
  `endpoint`, `database_table`, `job`, ...) held by the module-private
  `BOUNDARY_KINDS` set (`semanticAppearance.ts:57`). Step 3 exports that set
  (or an `isBoundaryKind` helper) — nothing else in the file changes.

- `packages/compass-viewer/src/graph/communityOverview.ts` labels the largest
  communities by member count only:

  ```ts
  // communityOverviewLabelledIds
  .sort((left, right) =>
    (right.memberCount ?? 0) - (left.memberCount ?? 0)
    || (right.degree ?? 0) - (left.degree ?? 0)
    || left.id.localeCompare(right.id))
  ```

- `packages/compass-viewer/src/graph/VisNetworkCanvas.tsx` renders the overview:
  `communityOverviewOptions` (physics off, `layout.improvedLayout: false`),
  `communityBubbleLabel(node)` = `"<name>\n<count> symbols"`, font
  `vadjust = size + 12` with a background-coloured halo, and
  `edgeAppearance(confidence, weight)` which gives `confidence === "aggregated"`
  edges `width = min(5, 1 + 1.5*log2(1+weight))` and
  `opacity = min(0.5, 0.16 + 0.09*log2(1+weight))`.

- `packages/compass-viewer/src/graph/CompassGraph.tsx` owns the derived view
  state: `derivedOverview`, `derivedDetail`, `scope` (`"communities" |
  "symbols"`), the `compass-scope-toggle` control, `focusSearchResult`, and the
  status line (`"<n> communities · <n> symbols"`).

- `packages/compass-viewer/src/graph/GraphInspector.tsx` renders the community
  panel (`CommunityControls`, `communityCounts`, `aggregatedSymbols`) and the
  footer (`"<n> communities · <n> symbols · <n> cross-community relationships"`).

- `packages/compass-viewer/src/graph/EdgeHoverCard.tsx` +
  `edgeLabels.ts` render an edge hover as
  `formatGraphEdgeLabel(edge)` = `"<relation> [<CONFIDENCE>]"`, optionally with
  `· file:line` from `relationshipSite`.

- Repo conventions for the viewer: TypeScript strict, React 19 function
  components, deterministic ordering (sort before publishing anything derived),
  CSS in `packages/compass-viewer/src/theme.css` using existing tokens
  (`--compass-line`, `--muted-foreground`, `--workbench-hover`,
  `--compass-focus`), tests beside implementation in `*.test.ts(x)` plus the
  Playwright specs in `tests/viewer`. Structural pattern to copy:
  `packages/compass-viewer/src/graph/communityOverview.test.ts` and
  `tests/viewer/community-overview.spec.ts`.

## Commands you will need

Run every command from the repository root. Rust is untouched by this plan, so
no `CARGO_TARGET_DIR` override is needed.

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Install JS deps (only if missing) | `npm ci` | exit 0 |
| Typecheck | `npm run typecheck:js` | exit 0, no errors |
| Viewer unit tests | `npm test -w @compass/viewer` | all pass, including new tests |
| Browser tests | `cd tests/viewer && npx playwright test --project=chromium` | all pass |
| Rebuild embedded viewer | `node scripts/build_viewer_assets.mjs` | writes `crates/compass-output/assets/viewer/{graph.js,viewer.css,manifest.json}` |
| Verify embedded viewer | `node scripts/check_viewer_assets.mjs` | `Viewer assets match the deterministic manifest` |

## Scope

**In scope** (the only files you may modify or create):

- `packages/compass-viewer/src/graph/communityOverview.ts`
- `packages/compass-viewer/src/graph/communityOverview.test.ts`
- `packages/compass-viewer/src/graph/CompassGraph.tsx`
- `packages/compass-viewer/src/graph/CompassGraph.community.test.tsx`
- `packages/compass-viewer/src/graph/VisNetworkCanvas.tsx`
- `packages/compass-viewer/src/graph/GraphInspector.tsx`
- `packages/compass-viewer/src/graph/GraphToolbar.tsx`
- `packages/compass-viewer/src/graph/semanticAppearance.ts` (export
  `BOUNDARY_KINDS` or add `isBoundaryKind`; no behaviour change)
- `packages/compass-viewer/src/graph/EdgeHoverCard.tsx` (only if the hover needs the breakdown)
- `packages/compass-viewer/src/theme.css`
- `tests/viewer/community-overview.spec.ts`
- `tests/viewer/fixtures/generate.ts` (only to add edge kinds to the existing `largeSymbolGraph` fixture)
- `crates/compass-output/assets/viewer/{graph.js,viewer.css,manifest.json}` (regenerated only — never hand-edited)

**Out of scope** (do NOT touch, even though they look related):

- Any Rust file, including `crates/compass-output/src/html.rs` — the exported
  aggregate wording is specified by Plan 027's typed hierarchy artifact. This
  plan must work with the model shapes that exist today.
- `packages/compass-viewer/src/contracts/*.ts` — do not add fields to
  `compass.viewer.graph/1`. Keep the relationship breakdown viewer-local.
- `packages/compass-viewer/src/graph/renderingProfile.ts` layout constants —
  packing geometry is already verified by `renderingProfile.test.ts`; changing
  it is a separate change.
- `editors/vscode/**` — the shared viewer is consumed there, but no
  extension-side change is required or permitted.

## Git workflow

- Branch: `codex/community-overview-legibility` (repo convention: `codex/` prefix).
- Commit style from `git log --oneline`: `fix bounded impact traversal on direct call chains`,
  `build: refresh viewer contract assets` — lowercase, imperative, optionally
  scoped. Suggested commits: one per step, with the regenerated assets committed
  together with the source that produced them.
- Do NOT push or open a pull request unless the operator instructed it.

## Steps

### Step 1: Keep the relationship mix when aggregating a community pair

In `communityOverview.ts`, change `aggregateCommunityEdges` to accumulate a
per-category count instead of a single number, using
`edgeSemanticCategory(edge.relation)` from `./semanticAppearance`:

```ts
type CommunityPair = {
  source: number;
  target: number;
  total: number;
  categories: Map<EdgeSemanticCategory, number>;
};
```

Keep `weight` = `total` (exact relationship count — do not change its meaning),
keep the undirected `(min,max)` pairing, and keep the deterministic ordering by
`(source, target)`. Render the relation text as a stable, human-readable mix,
dominant category first, ties broken by the category order in
`EDGE_SEMANTIC_CATEGORIES`:

```text
12 calls · 4 imports          // relation text, exact counts, no invented kinds
```

Use the existing kind label mapping style from `edgeLabels.ts` (verbs such as
`calls`, `imports`, `contains` — preserve the raw relation string; do not
translate it into a category word if the relation itself is already readable).
When two relations share the dominant count, join both; when a pair has more
than three distinct relations, keep the three largest and append
`· <n> more`. Expose the dominant category so the canvas can colour by it:
return it from the aggregation and thread it through `communityOverviewModel`
as an internal map the viewer owns (for example
`export type CommunityOverview = { model: GraphViewModel; edgeCategories: ReadonlyMap<string, EdgeSemanticCategory> }`).

**Verify**: `npm test -w @compass/viewer -- communityOverview` → all pass after
Step 4's tests are added; before that, `npm run typecheck:js` → exit 0.

### Step 2: Colour aggregated edges by the dominant relationship category

In `VisNetworkCanvas.tsx`, give aggregated edges the category colour that
`semanticEdgePalette` already provides, while keeping the weight-driven width
and opacity from `edgeAppearance`:

- `communityOverview` models: `color: semanticEdgePalette[dominantCategory]`.
- Non-aggregated models: unchanged.

Thread the category map into `VisNetworkCanvas` through a new optional prop
(for example `edgeSemanticHints?: ReadonlyMap<string, EdgeSemanticCategory>`)
that `CompassGraph.tsx` passes only for the derived overview. Do not change the
rendering of Rust-exported aggregated models here.

**Verify**: `npm run typecheck:js` → exit 0.

### Step 3: Rank labels by importance, not size alone

Compute an importance score inside `communityOverview.ts` and use it for the
label budget (and for the inspector's panel order):

```text
importance = log2(1 + memberCount)
           + 1.5 * log2(1 + interCommunityDegree)
           + 2.0 * log2(1 + boundaryMemberCount)
```

`boundaryMemberCount` counts members whose `kind` is in the existing
`BOUNDARY_KINDS` set from `semanticAppearance.ts` (routes, endpoints, jobs,
database objects — the things a reader navigates by); export that set or an
`isBoundaryKind(kind)` helper so the module can reuse it. Keep
`communityOverviewLabelLimit` as the budget, keep the current truncation
(`COMMUNITY_LABEL_MAXIMUM_CHARACTERS`), and keep the result deterministic
(`importance desc`, then `memberCount desc`, then `id asc`).

Add a second visual tier so the long tail stops competing with the labelled
set: labelled bubbles keep a filled dot; unlabelled bubbles render at
`opacity 0.82` (already implemented) and with `borderWidth` equal to the
labelled value so the map still reads as one system.

**Verify**: `npm test -w @compass/viewer -- communityOverview` → ordering and
budget tests pass.

### Step 4: Write the failing-first tests for Steps 1–3

Extend `communityOverview.test.ts` (structural pattern: the existing
`counts cross-community relationships without inventing direction` test):

- a pair with 3 `calls` and 1 `imports` yields
  `relation === "3 calls · 1 imports"`, `weight === 4`, and dominant category
  `execution`;
- more than three distinct relations keep the three largest and append
  `· <n> more`;
- identical input in reversed order yields an identical model (compare the
  whole derived model, as the existing determinism test does);
- a community with the same member count but more boundary members and more
  inter-community degree is labelled ahead of a purely larger one;
- the label budget still holds (`communityOverviewLabelLimit(nodes.length)`).

**Verify**: `npm test -w @compass/viewer` → all pass, including the new tests.

### Step 5: Add the breadcrumb and the panel's open action

In `CompassGraph.tsx` add a breadcrumb to the graph stage that reflects the
derived scope:

```text
Repository ▸ <Community label>            (derived detail)
Repository                                (derived overview)
```

Requirements:

- render it as a `<nav aria-label="Graph path">` with a button per ancestor;
  clicking an ancestor is exactly the existing "Back to community overview"
  action, so reuse `backToOverview`;
- keep the existing status text and scope toggle unchanged;
- `Escape` already returns to the overview — keep that behaviour and mention the
  breadcrumb in the graph settings help text (`GraphToolbar` settings panel)
  only if that panel already documents keyboard controls.

In `GraphInspector.tsx`'s community panel:

- sort rows by the importance score from Step 3, descending, and mark the
  labelled set with the existing colour dot only (no new chrome);
- add an "Open group" button per row that calls the same handler the canvas
  uses for a community bubble (`host.openCommunity`), so a keyboard user can
  drill in without the canvas;
- keep the filter input, "Select all", and the bounded list behaviour intact.

**Verify**: `npm run typecheck:js` → exit 0, and
`cd tests/viewer && npx playwright test --project=chromium community-overview.spec.ts`
→ pass.

### Step 6: Extend the browser spec and the fixture

In `tests/viewer/fixtures/generate.ts`, give `largeSymbolGraph` edges their real
kind mix instead of one `calls` string: use at least three distinct relation
strings (`calls`, `imports`, `references`) spread across cross-community edges,
keeping the existing deterministic construction.

In `tests/viewer/community-overview.spec.ts` add one test that asserts:

- at least one aggregated edge label in the hover card contains two distinct
  relation verbs and two exact counts;
- the breadcrumb shows `Repository` in the overview and
  `Repository ▸ <community>` after selecting a search result, and the
  breadcrumb button returns to the overview;
- the first community row in the panel has an enabled "Open group" button that
  opens the same community as the canvas.

**Verify**: `cd tests/viewer && npx playwright test --project=chromium` → all pass.

### Step 7: Rebuild and verify the embedded viewer assets

```bash
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
```

**Verify**: the check prints `Viewer assets match the deterministic manifest and
use no remote runtime resources.` and `crates/compass-output/assets/viewer/` +
`manifest.json` are modified in `git status`.

## Test plan

- New unit tests in `communityOverview.test.ts` (relationship mix, dominant
  category, deterministic ordering, importance ranking, label budget).
- New assertions in `CompassGraph.community.test.tsx` (breadcrumb present in
  overview and detail, "Open group" opens the community).
- New Playwright test in `tests/viewer/community-overview.spec.ts` on the
  extended `largeSymbolGraph` fixture.
- Existing suites must stay green: `renderingProfile.test.ts`,
  `VisNetworkCanvas.interaction.test.tsx`, `GraphInspector.render.test.tsx`,
  `tests/viewer/performance.spec.ts`.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `npm run typecheck:js` exits 0
- [ ] `npm test -w @compass/viewer` exits 0 and includes the new
      relationship-mix, importance, and breadcrumb tests
- [ ] `cd tests/viewer && npx playwright test --project=chromium` exits 0
- [ ] `grep -n "cross-community edges" packages/compass-viewer/src/graph/communityOverview.ts`
      returns no match (the derived overview no longer emits the opaque count string)
- [ ] `node scripts/check_viewer_assets.mjs` exits 0 and the only regenerated
      files are `crates/compass-output/assets/viewer/{graph.js,viewer.css,manifest.json}`
- [ ] `git status --short` shows no Rust source, contract, or fixture file
      outside the in-scope list
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- `communityOverview.ts` no longer contains `aggregateCommunityEdges` with the
  `"<count> cross-community edges"` shape (the codebase drifted from this plan).
- The relationship kind is not available on `GraphEdge` for the models you must
  support, or an edge kind cannot be mapped without inventing a category.
- Making the hover show the mix requires a contract change in
  `packages/compass-viewer/src/contracts/`.
- A step's verification fails twice after a reasonable fix attempt.
- You are asked to change packing geometry, physics defaults, or the Rust
  aggregate wording to make a visual detail work — those are separate plans.

## Maintenance notes

- The derived relationship mix is presentation-only and only exists for models
  the viewer aggregates itself. When Plan 027 lands, exported aggregated models
  will carry typed per-category weights; at that point this module must read the
  typed field and keep the local computation only as the fallback for older
  exports. Do not let both representations drift into different wording.
- `weight` must keep meaning "exact number of relationships between this
  community pair". Any future feature that wants a normalized strength must add
  a separate field rather than rescaling `weight`.
- Reviewers should check: the mix text never truncates counts, the dominant
  category is deterministic under input reordering, and the label budget is
  still bounded for repositories with thousands of communities (zod-class
  inputs).
- Follow-up explicitly deferred: per-level navigation and typed hierarchy
  fields (Plan 027), stable group ids across builds (Plan 028).
