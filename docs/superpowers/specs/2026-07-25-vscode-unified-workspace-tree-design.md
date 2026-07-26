# VS Code Unified Workspace Tree

Date: 2026-07-25

## Objective

Replace Compass's separate Repository and Operations sidebar views with one
minimal, reliable, VS Code-native Workspace view. Each workflow must have one
obvious sidebar location, repository state must remain visible, and active or
recoverable work must appear only when relevant.

## Problem

The current Compass activity-bar container contributes two tree views:
Repository and Operations. Repository exposes Open graph and Codebase evolution
under every available repository. Operations exposes those workflows again,
while the Repository title bar repeats both shortcuts. The same action therefore
appears in as many as three places.

The duplication weakens hierarchy, makes Repository and Operations compete for
the same purpose, and creates uncertainty about whether similarly named entries
behave differently. The underlying commands are already centralized and reliable;
the problem is the sidebar information architecture.

## Scope

This change:

- replaces Repository and Operations with one Workspace tree;
- removes duplicate graph and history shortcuts from the view title;
- keeps repository status visible without attaching routine workflow children;
- shows Explore and Maintain groups only once;
- shows active operations only while they exist;
- adds a read-only Workspace refresh command;
- preserves current command handlers, trust checks, capability checks, repository
  selection, and Command Palette access;
- updates extension documentation and automated coverage.

This change does not redesign editor webviews, alter Compass CLI contracts, add
custom sidebar HTML, or change graph/history execution behavior.

## Information Architecture

The Compass activity-bar container contributes one view named `Workspace`.
Its stable view identifier remains `compass.status` so VS Code can preserve
existing placement and view state across the extension update.

For a healthy repository with a graph, the view renders:

```text
WORKSPACE

codegraph                         Graph ready

ACTIVE OPERATIONS                 only while work is active
  Building graph                  codegraph
  Watching for changes            codegraph

EXPLORE                           expanded
  Code graph
  Architecture flow
  Call graph from cursor
  Ask codebase
  Codebase evolution

MAINTAIN                          collapsed
  Update graph
  Watch for changes
```

Repository rows are status indicators and context, not duplicate workflow
containers. Multiple workspace repositories appear as separate status rows.
Global workflow actions continue to resolve the active editor's repository
first, use the only repository when exactly one exists, and show the existing
repository picker when the target is ambiguous.

### Ordering

Top-level items appear in this order:

1. CLI attention or setup, when required.
2. Repository status rows.
3. Active operations, when present.
4. State-specific initialization or recovery action, when required.
5. Explore.
6. Maintain, when maintenance actions are available.

The order keeps blockers and ongoing work above routine navigation.

## State Behavior

### Healthy graph

An available repository displays `Graph ready`. Explore is expanded and exposes
all five exploration workflows. Maintain is collapsed and exposes Update graph
and the contextual watch action.

### Graph not initialized

The repository displays `Not initialized`. A single prominent Initialize
repository action appears after repository status. Graph-dependent Explore
commands are omitted. Codebase evolution remains available because browsing Git
history does not require a current materialized graph.

### Graph building

The repository displays `Building`. Active operations appears expanded with the
current build. Conflicting build actions are omitted while the writer is active.

### Graph build failed

The repository displays `Build failed`. A single Retry graph build recovery
action appears after repository status. Codebase evolution remains available.

### Watch active

Active operations appears expanded with Watching for changes. Maintain changes
its watch action to Stop watching. Active-operation rows are status-only so the
start/stop workflow appears exactly once.

### CLI missing or incompatible

A single CLI attention row appears first with a concise state description and
opens the existing setup flow. Normal workflow groups are hidden until the CLI
is usable, avoiding controls that can only fail.

### No repository

The tree shows one quiet empty-state action instructing the user to open a
repository folder. It does not render empty Explore or Maintain groups.

## Labels and Visual Rules

The view uses native VS Code tree rendering and `ThemeIcon` symbols exclusively.
It adds no custom colors, cards, badges, separators, or webview styling.

Canonical visible labels are:

- Workspace
- Active operations
- Explore
- Maintain
- Code graph
- Architecture flow
- Call graph from cursor
- Ask codebase
- Codebase evolution
- Initialize repository
- Retry graph build
- Update graph
- Watch for changes
- Stop watching

Repository state descriptions are:

- Graph ready
- Not initialized
- Building
- Build failed

Short descriptions remain visible only when they add context, such as the
repository name beside an active operation. Longer explanations remain in
tooltips. Command Palette titles retain verb-led `Compass:` names.

## View Title

The Workspace title bar exposes only `Refresh Compass Status`. This command
refreshes repository/session state and tree presentation. It does not initialize,
update, watch, or otherwise build a graph.

Open graph, Codebase evolution, and Update graph are removed from the view-title
menu because they duplicate visible tree workflows or imply a graph build from a
generic refresh icon.

## Architecture

The extension continues to use a pure descriptor model:

- `buildWorkspaceTree(discovery, sessions)` produces the entire Workspace tree.
- A single Workspace tree provider converts descriptors to native
  `vscode.TreeItem` instances.
- Repository sessions remain the source of truth for graph state, active writers,
  and watchers.
- Existing command handlers remain the only execution path for builds, graphs,
  architecture, calls, queries, and history.
- Existing command-time trust, compatibility, and graph-state checks remain
  authoritative if state changes after the tree renders.

The `compass.operations` view contribution, Operations tree provider, and separate
Operations builder are removed. The stable `compass.status` identifier is retained
and its provider becomes the unified Workspace provider.

`compass.refreshWorkspace` calls the existing registry refresh path and refreshes
the Workspace tree and status bar. It never invokes a writer command.

## Multi-root Workspaces

All repository status rows remain visible. Tree workflows are listed once rather
than repeated under every repository.

When an action is invoked:

1. An explicit repository argument wins.
2. The repository containing the active editor is used.
3. The only repository is used when there is exactly one.
4. Otherwise the existing repository picker asks the user.

No new persistent repository selector is introduced.

## Accessibility and Reliability

- Native tree semantics provide keyboard navigation, focus, expansion, high
  contrast, and screen-reader behavior.
- Groups use clear text labels in addition to icons.
- Animated operation icons remain limited to actual active work.
- Tooltips explain failures and recovery actions without crowding the tree.
- Commands validate current state when invoked and do not trust potentially stale
  tree presentation.
- Initialization and retry actions restrict repository selection to repositories
  in the corresponding missing or failed state.
- The refresh action is explicitly read-only.

## Documentation

The extension README will describe one Workspace view instead of separate
Repository and Operations sections. Each workflow will document one sidebar
location plus Command Palette availability. References to opening Codebase
Evolution from multiple duplicate locations will be removed.

## Verification

Implementation proceeds directly rather than using a test-driven-development
sequence. Automated coverage is added after implementation and must verify:

- one contributed Compass view;
- no duplicate command within a Workspace tree snapshot;
- canonical group ordering and labels;
- healthy, missing, building, failed, watch-active, CLI-attention, empty, and
  multi-root states;
- read-only refresh registration and behavior;
- preservation of every existing primary command.

Final verification includes:

- VS Code extension unit tests;
- TypeScript type checking;
- production viewer/extension build;
- extension-host integration checks where supported;
- VSIX packaging and smoke validation;
- repository-required `graphify update .`.

## Non-goals

- A custom sidebar webview.
- A dashboard or welcome page.
- Automatic historical graph materialization.
- New CLI or graph contracts.
- Persistent repository selection.
- Changes to Query, Architecture, Call Graph, Graph, or Codebase Evolution editor
  tabs.
