# VS Code Trust and Correctness Design

**Date:** 2026-07-25

**Status:** Approved scope; implementation contract for PR 1

## Context

Compass Codebase Evolution currently keeps the selected commit inside
`HistoryWorkspace`, while the webview adapter separately owns the loaded graph,
comparison, community detail, and change counts. Selecting a new commit therefore
does not invalidate the previously loaded revision. A late response for an older
selection can also repaint the graph after the user has moved elsewhere.

Historical builds have a second split-brain state. The React component marks a
commit as building before asking the extension host to choose a profile. The host
does not send a terminal build event when profile selection is cancelled, a build
fails, or a build succeeds. A failed or cancelled build can consequently leave
the action disabled until the panel is reopened.

Architecture and call-graph views also hide bounded output. Architecture renders
only the first 24 overview flows, call graph renders only the first 20
continuations, and the call-graph `truncated` contract field is not presented.
These omissions undermine trust because a partial result looks complete.

## Goal

Make every displayed historical graph provably match the selected commit, make
every historical build reach a visible recoverable terminal state, and make every
bounded architecture or call-graph result disclose its incompleteness.

## Non-goals

- Redesigning the complete History, Architecture, or Call Graph surfaces.
- Adding webview serializers or cross-reload persistence.
- Changing CLI graph limits or materialization semantics.
- Adding background or implicit historical builds.
- Optimizing extension activation or bundle size.
- Adding telemetry.
- Creating or using a `graphify-out/` runtime path. Compass artifacts remain
  under `compass-out/`.

## Considered approaches

### 1. Controlled history state in the webview adapter — selected

The webview adapter owns the selected commit and all revision-bound presentation
state. `HistoryWorkspace` receives the selected commit, build state, and errors as
props. Selecting a commit synchronously clears graph, comparison, community, and
change-count state. Incoming graph, comparison, community, and change-count
messages are accepted only when their commit still matches the selection.

This approach fixes both ordinary selection changes and late-response races while
keeping CLI orchestration in the extension host.

### 2. Reset `HistoryWorkspace` with a React key

Remounting the workspace on selection or timeline changes would clear local state
with little code. It would not prevent the webview adapter from accepting an old
host response, and it would discard unrelated search state. This is insufficient.

### 3. Move all history presentation state into the extension host

The extension host could become the single state machine and send complete view
snapshots. This would be robust but would substantially expand the PR into panel
serialization and lifecycle architecture. It is deferred.

## Historical selection model

The webview adapter owns:

- `selectedCommit`;
- the loaded `graph` and its `graphCommit`;
- comparison output;
- change counts;
- community detail and its active request;
- build state keyed by commit;
- operation errors keyed by commit.

When a timeline first arrives, the adapter selects `selectedHead` when it is
present in the entries, otherwise the first entry. A later timeline refresh keeps
the current selection if it still exists.

Selecting another commit performs one synchronous transition:

1. set `selectedCommit`;
2. clear graph and graph identity;
3. clear comparison output;
4. clear change counts;
5. clear community detail, loading state, and active request;
6. render the selected commit's empty graph state.

Search text remains component-local and is not cleared by selection.

Host responses for graph load, comparison, community detail, and change counts
carry commit identity. The adapter ignores a response whose commit does not equal
`selectedCommit`. An ignored response does not alter visible errors or loading
state for the current commit.

When a graph is visible, the workspace shows a context strip containing the short
commit identity. The strip is presentation evidence, not the source of truth.

## Historical build lifecycle

Each commit can have one of these webview build states:

- `requesting`: the user requested a build and the host is gathering a profile;
- `running`: a Compass process is active;
- `failed`: the last request failed and can be retried;
- no state: idle or successfully completed.

The webview marks a commit `requesting` immediately when Build graph is selected.
The extension host must answer every accepted `buildRevision` request with a
terminal or running event:

- `buildRunning` after the CLI process starts;
- `buildSucceeded` after the refreshed timeline has been posted;
- `buildFailed` with a concise message on process, validation, or refresh failure;
- `buildCancelled` when profile selection, profile-source entry, progress
  cancellation, or process cancellation prevents completion.

`buildRunning` may include the latest JSONL phase message, but progress expansion
is not required in this PR.

`requesting` and `running` disable Build graph. `failed`, `cancelled`, and
`succeeded` make it actionable again. Failure copy appears directly below the
selected commit actions. Cancellation is neutral and does not render as a
semantic comparison error.

Generic history operations send an error with an operation name and commit when
available. Load and comparison errors appear beside the selected commit rather
than being placed inside semantic findings. Community errors remain in the graph
because that is the surface that initiated them.

When a parent revision is not presentation-available, Compare parent is disabled
and explains that both revision graphs must be built first.

## Truncation disclosure

### Architecture

Architecture continues to render at most 24 overview flows initially. When more
exist, the view shows `Showing 24 of N flows` and a Show all action. Choosing Show
all renders the complete contract result. No result is dropped without a visible
count.

### Call graph

The call-graph status panel always shows rendered node and edge counts. When
`graph.truncated` is true, it renders an alert explaining that the graph reached
its configured size boundary and that continuations expand the bounded result.

At most 20 continuation actions are shown initially. If more exist, the panel
shows `Showing 20 of N continuations` and a Show all action. Expansion preserves
the merged `truncated` value already provided by the call-graph state helper.

## Error and race handling

- Selecting a commit while a prior graph or comparison is loading invalidates
  the prior response by identity.
- Selecting another commit while a build runs does not cancel the build. Its
  state remains keyed to the original commit and is visible again if reselected.
- A successful build refreshes the timeline before clearing the build state.
- A timeline refresh never silently moves selection when the selected commit
  still exists.
- A failed timeline refresh sends `buildFailed`; it does not report success just
  because the CLI process exited successfully.
- Disposing the panel continues to terminate work through existing controllers
  and process cancellation boundaries.

## Acceptance criteria

1. Selecting commit B after opening commit A immediately removes A's graph,
   comparison, community detail, and counts.
2. A delayed graph or comparison response for A cannot become visible while B is
   selected.
3. Every visible historical graph includes the selected short commit identity.
4. Cancelling profile selection restores Build graph without an error.
5. Cancelling, failing, or successfully completing a historical build restores a
   usable action state without reopening the panel.
6. Build and load failures appear beside the selected commit and do not masquerade
   as semantic findings.
7. Compare parent is disabled with an explanation when either graph is missing.
8. A truncated call graph visibly reports that it is partial and shows rendered
   node and edge counts.
9. More than 20 call continuations expose their total and can all be revealed.
10. More than 24 architecture flows expose their total and can all be revealed.
11. Existing community drilldown, source navigation, explicit-build behavior,
    reduced motion, and local-only webview behavior remain intact.

## Verification approach

This PR does not use a red/green TDD workflow. Implementation comes first,
followed by focused verification:

- viewer/browser scenarios for revision selection, stale response rejection,
  build terminal states, comparison availability, architecture disclosure, and
  call-graph disclosure;
- TypeScript type checks for the viewer and VS Code extension;
- existing viewer and extension unit suites;
- Chromium viewer qualification;
- VS Code integration tests where the fake CLI boundary covers the changed host
  behavior;
- production builds and VSIX smoke qualification.

No acceptance criterion is considered complete until the corresponding
post-implementation check has passed.
