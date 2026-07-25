# Compass VS Code Extension Design

**Date:** 2026-07-24

**Status:** Approved

**Implementation root:** `/Users/haipingfu/graphify/compass`

## Purpose

Compass will provide a first-party Visual Studio Code extension for understanding
and operating on a codebase without leaving the editor. The extension will
preserve the interaction model of Compass's exported code graph, add an
editor-aware symbol call graph, expose the broader architecture call-flow
document, make common Compass CLI workflows approachable, and show how the
codebase evolves across every Git commit.

The extension requires a separately installed `compass` CLI. It guides users
through installation and upgrade but does not bundle platform-specific Compass
binaries. Compass remains the authority for graph construction, history,
queries, diffs, and program evidence.

All capabilities in this design are required for the first complete release.
The delivery phases are implementation milestones, not optional follow-up
features.

## Product principles

1. **Compass remains authoritative.** The extension consumes public artifacts
   and versioned CLI contracts. It never reads Compass's history SQLite schema.
2. **One graph experience.** Exported Compass HTML and the VS Code extension
   use the same React graph components, interaction state, and design tokens.
3. **Native to VS Code.** Navigation, notifications, output, progress,
   commands, workspace trust, and source opening follow VS Code conventions.
4. **Uncertainty stays visible.** Partial Program IR coverage, inferred calls,
   unavailable history, and stale graphs are not presented as exact knowledge.
5. **Expensive work is explicit.** Viewing status or selecting a missing
   historical commit never silently materializes a graph.
6. **Local-first and private.** The viewer makes no runtime network requests,
   sends no telemetry, and does not expose source-derived graph data.

## Goals

- Guide users through locating, installing, validating, and upgrading the
  Compass CLI.
- Provide purpose-built interfaces for `init`, `update`, `watch`, current graph
  exploration, symbol call graphs, architecture flow, natural-language query,
  CompassQL, history materialization, and semantic diff.
- Preserve the stable-layout, relationship-spotlight, search, filtering,
  inspection, and accessibility behavior of Compass's current `graph.html`.
- Open source files and exact locations from graph nodes and query results.
- Start a call graph from the symbol under the editor cursor and expand callers
  or callees on demand.
- Display every Git commit with its Compass graph state and support explicit
  materialization of missing revisions.
- Compare a revision with a selected parent while preserving shared node
  positions and selection where possible.
- Work in multi-root workspaces and on remote extension hosts such as Remote
  SSH, WSL, and Dev Containers.
- Package and verify a Marketplace-ready VSIX for macOS, Linux, and Windows.

## Non-goals

- Bundling or independently updating the Compass native binary.
- Supporting browser-only `vscode.dev` workspaces.
- Reading or mutating private Prolly or SQLite tables from TypeScript.
- Reimplementing graph extraction, query, diff, or history semantics in the
  extension.
- Automatically materializing every commit when a user opens the history view.
- Preserving the data model of the unrelated vendored
  `code-review-graph-vscode` project.
- Providing a generic graphical form for every advanced Compass flag. The
  extension focuses on the approved common workflows; advanced users retain
  the integrated terminal and full CLI.

## Architecture

The system has three explicit layers:

```text
Compass Rust core and CLI
          ↓ versioned JSON, JSONL events, and public artifacts
VS Code extension host
          ↓ validated typed webview messages
Shared React viewer package
```

### Shared viewer package

Add `packages/compass-viewer`, built with React, TypeScript, Tailwind CSS,
shadcn/ui primitives, and Lucide icons. It owns:

- graph canvas adapters and shared graph interaction state;
- current-graph and historical-graph presentation;
- symbol call-graph presentation and expansion state;
- architecture call-flow presentation;
- commit timeline and comparison presentation;
- inspectors, filters, search, status, loading, and error surfaces;
- theme tokens, accessibility behavior, and responsive layouts; and
- runtime schema types shared with the extension host.

The graph renderer remains behind a small adapter interface so it can be
upgraded without changing product state. The first version keeps the proven
Compass/vis-network behavior, bundles the dependency locally, and removes
runtime CDN dependence.

### VS Code extension

Add `editors/vscode` as the first-party extension package. Its Node-based
extension host owns:

- workspace-folder and repository sessions;
- installed CLI discovery and capability negotiation;
- safe process execution, streaming, cancellation, and watch lifecycle;
- public artifact discovery and freshness monitoring;
- extension-managed historical export storage;
- source-path and source-location resolution;
- webview creation, restoration, and typed message routing;
- activity-bar, status-bar, command, context-menu, and output-channel
  integration; and
- Marketplace packaging and extension-host tests.

Webviews never spawn processes or read the filesystem directly. They request
operations through typed messages to the extension host.

### Compass output integration

`compass-output` embeds prebuilt viewer assets into generated graph and history
HTML. End users do not need Node.js to run Compass or open exports. Node is
needed only by contributors and release automation when rebuilding viewer
assets.

The Rust renderer continues to own graph serialization, output safety, atomic
publication, and offline HTML assembly. The shared viewer owns presentation and
interaction. Checked-in or release-generated assets have a deterministic
manifest so stale JavaScript cannot be shipped accidentally.

### Authority boundary

The extension may read documented public artifacts such as `graph.json`,
`program.json`, and reports. It invokes `compass` for all stateful or semantic
operations. It does not link Rust crates into the extension, duplicate Compass
algorithms in TypeScript, or inspect the history database.

## Identity, visual language, and theming

The extension uses Lucide's Compass mark as the basis of its activity-bar and
Marketplace identity, subject to the Lucide license and VS Code asset
requirements. Monochrome sidebar artwork uses the current VS Code foreground;
the Marketplace icon receives a restrained Compass-branded treatment while
remaining recognizable at small sizes.

The interface follows VS Code information architecture and maps its semantic
tokens to VS Code CSS variables, including editor backgrounds, input borders,
focus rings, selection, errors, warnings, links, and high-contrast outlines.
Tailwind and shadcn/ui supply component structure rather than imposing a
separate website aesthetic. Compass community colors remain the primary graph
data colors.

The viewer supports VS Code light, dark, and high-contrast themes. It honors
reduced motion, reduced transparency where available, keyboard navigation, and
screen-reader status announcements.

## VS Code information architecture

### Compass activity-bar container

The Compass container provides a compact native sidebar with:

- selected repository and workspace folder;
- installed CLI version and compatibility;
- current revision and dirty-worktree indicator;
- graph freshness and artifact counts;
- watch and history state;
- primary actions for initialization, update, watch, graph, calls,
  architecture, query, and history; and
- recent operation states.

Simple lists and actions use native VS Code tree views, Quick Picks, input
boxes, progress, and notifications. Rich graph and timeline experiences open
in editor tabs.

### Status bar and output

A concise Compass status-bar item shows the selected repository's state and
opens the Compass sidebar. It must not animate continuously or show routine
success notifications.

All command output is available in a dedicated Compass output channel. Guided
views show structured progress and actionable errors. Notifications are
reserved for required decisions, completion of user-initiated background work,
and failures.

## Guided setup and CLI lifecycle

On activation, the extension locates `compass` in this order:

1. a workspace-appropriate explicit configuration;
2. the extension-host `PATH`; and
3. a previously validated machine-scoped path.

The extension runs the binary directly with argument arrays. It never constructs
a shell command string. A capability handshake returns the CLI version,
machine-contract versions, repository support, and available features.

When Compass is missing, the welcome view explains the requirement and offers
copyable official installation commands plus a binary-path picker. The
extension does not install software without a separate explicit user action.
When Compass is present but incompatible, the UI shows the installed and
required versions and links to upgrade guidance.

Command execution is disabled in untrusted workspaces. In Remote SSH, WSL, and
Dev Containers, detection and execution occur on the remote extension host,
where the repository files reside.

## Current graph experience

The current graph opens in an editor tab and uses the same shared viewer as
Compass's exported `graph.html`. It preserves:

- initial physics followed by automatic stabilization;
- explicit pause/resume, fit, reset, and label controls;
- one focus path for canvas, search, and neighbor selection;
- relationship spotlighting that keeps a selected node and its direct
  neighborhood vivid while dimming unrelated content;
- node search with keyboard navigation;
- community visibility filters and community overview;
- graph statistics and graph-state announcements;
- a selected-node inspector with type, source, community, degree, and
  connected nodes; and
- narrow-layout and reduced-motion behavior.

VS Code adds source navigation. Selecting `Open source` resolves the node's
document URI and location through the extension host, opens the editor, and
reveals the most precise available range. Missing, moved, generated, or
out-of-scope files produce an explanation without breaking graph selection.

A stale graph remains viewable with a prominent freshness warning and actions
to update or start watch mode. A graph above 5,000 nodes opens in community
overview mode while retaining exact nodes for search and focused expansion.

## Symbol call graph

`Compass: Show Call Graph` resolves the innermost Program IR function at the
active cursor. If multiple symbols match, a Quick Pick shows names, signatures,
and source ranges. If no function covers the cursor, the user may search for a
symbol.

The call graph opens with the selected symbol as its root and supports:

- callers, callees, or both directions;
- expand-on-demand with a configurable depth bound;
- collapse back to the root path;
- breadcrumbs and a persistent root indicator;
- direct, inferred, ambiguous, and unresolved call presentation;
- parallel call sites without losing evidence;
- source navigation for functions and individual call evidence;
- caller/callee counts and filtering; and
- capability coverage explanations by language and construct.

Resolved calls use directional edges. Inferred and ambiguous calls use distinct
non-color styling as well as labels. Unresolved calls terminate at explicit
unresolved nodes that show the available call expression and evidence. Missing
or partial capability coverage is never interpreted as proof that no calls
exist.

Expansion is lazy and cancellable. The host requests only the requested
neighborhood from Compass and the webview merges schema-compatible results into
its current state.

## Architecture call-flow document

A separate `Compass: Open Architecture Flow` editor tab presents the broader
project-oriented call-flow view. It includes:

- section navigation and architecture overview;
- subsystem diagrams derived from communities;
- per-section call diagrams and call tables;
- node, edge, hyperedge, confidence, and community statistics;
- report highlights and provenance; and
- source navigation where the model contains locations.

Compass Rust code remains responsible for deriving normalized call-flow
sections and evidence from the graph. It emits a versioned presentation model.
The shared React viewer renders that model both in VS Code and in the
`callflow-html` export. No remote Mermaid or font dependency is permitted at
runtime.

## Query experience

The Query tab supports two explicit modes:

- natural-language Compass graph discovery; and
- CompassQL with an editor, parameters, revision selection, limits, and result
  format.

Results render as structured tables, paths, nodes, or raw JSON as appropriate.
Rows that contain node IDs or source evidence provide graph focus and source
navigation actions. Query history is local to the workspace and does not store
credentials or full results unless the user explicitly saves them.

Current-tree queries use the current graph. Historical queries require a
selected available realization and preserve its exact commit and realization
identity in the result header.

## History and codebase evolution

The History tab shows every Git commit, not only materialized commits. A
lane-based, virtualized timeline presents:

- short SHA, subject, author, date, and parent lanes;
- the selected branch or revision context;
- graph state: available, missing, queued, building, failed, corrupt, or
  incompatible;
- preferred realization identity and extraction fingerprint when available;
- concise graph-change counts when comparable data exists; and
- filters for text, branch reachability, graph state, and date.

Reading the timeline never enables history or materializes a revision.
Selecting a missing commit shows metadata and an explicit `Build graph` action.
The guided build form exposes the approved history profile choices, including
reuse with `--profile-from`. Progress, cancellation, retry, and final validation
are visible in the timeline.

Selecting an available commit loads its exact validated graph through a
revision-specific history export in extension-managed temporary storage. The
viewer verifies the commit and realization identities before replacing the
canvas. It never substitutes another revision after an error.

The history viewer retains at most three decoded revision graphs. Shared node
IDs keep positions and selection across adjacent commits where possible.
Document state records the selected full SHA so restored tabs remain auditable.

### Compare with parent

`Compare with parent` uses the first parent by default and requires a parent
choice for merge commits when more than one is available. Comparison is
disabled with an explanation when the parent graph is missing or extraction
profiles are incompatible.

The comparison view keeps the complete selected-revision graph as its base and
overlays:

- added, removed, and changed nodes;
- added, removed, and changed edges;
- affected callers and modules;
- ranked semantic findings;
- verification and test evidence; and
- collapsed routine churn with an explicit expansion control.

Shared nodes retain layout positions. Removed nodes remain inspectable in a
clearly historical treatment. Selecting a semantic finding focuses the
supporting graph neighborhood and source evidence.

## Purpose-built Compass workflows

The extension supplies guided interfaces for:

- `compass init`, including repository scope preview, includes, excludes,
  confirmation, and existing-configuration handling;
- `compass update`, including output selection and common structural options;
- `compass watch`, including backend, debounce, state, retry, and stop;
- current graph exploration;
- symbol call graph and architecture flow;
- natural-language query and CompassQL;
- history enable, disable, status, build, list, show, prefer, and garbage
  collection where appropriate;
- history graph loading and export; and
- semantic diff, including parent comparison and finding explanation.

Destructive or storage-reclaiming actions retain explicit confirmation.
Advanced command flags remain available through the integrated terminal and
linked CLI help rather than being hidden behind an unmaintainable generic form.

## Machine contracts

Existing documented JSON contracts remain in use. Add only the missing
interfaces needed for a robust extension:

### Capability handshake

Add a machine-readable capability command that reports:

- Compass version;
- supported extension protocol versions;
- current graph, Program IR, query, diff, history, progress-event, call-graph,
  and call-flow schema versions;
- compiled optional features; and
- repository detection and platform limitations.

The extension rejects unknown major versions and tolerates additive minor
fields.

### Timeline

Add `compass history timeline --format json` as an inspection-only command. It
combines Git commit metadata with history-store state without enabling history,
creating a store, or materializing a graph. It represents rewritten or
partially unavailable history explicitly.

### Symbol call graph

Add a structured Program IR call-graph command that accepts a symbol or source
position, direction, depth, and bounds. Its result includes functions, calls,
resolution state, evidence, capability coverage, truncation, and continuation
information.

### Architecture flow model

Add a versioned structured call-flow presentation model generated from the same
Rust derivation used by `callflow-html`. The HTML exporter embeds that model
for the shared viewer.

### Progress events

Long-running guided operations expose versioned JSONL events with:

- operation and repository identity;
- phase, current item, totals, and optional byte or commit progress;
- severity and human-readable detail;
- retry and cancellation state; and
- exactly one terminal success, failure, or cancellation event.

The extension validates the process exit code and terminal event before
refreshing artifacts.

## Process and concurrency model

Each repository has an extension-host session with one coordinated writer
operation. Read-only queries may run concurrently within conservative bounds.
History maintenance, current graph builds, and watch-triggered updates cannot
race each other.

The process manager:

- passes arguments without shell interpolation;
- sets explicit working directories and environment overrides;
- streams stdout and stderr without unbounded buffering;
- redacts known secret-bearing configuration before logging;
- supports cancellation and graceful termination followed by bounded forced
  termination;
- owns one persistent watch process per repository;
- distinguishes user cancellation from command failure; and
- refreshes artifacts only after validated success.

Filesystem watchers observe public artifact replacement and configuration
changes. They debounce refreshes and never treat a partially staged file as a
published graph.

## Multi-root and repository selection

Each workspace folder may contain zero, one, or nested Git repositories.
Compass actions always show or remember an explicit repository target. The
status bar reflects the repository associated with the active editor, falling
back to the last explicit selection.

Commands that change state never run against an ambiguous repository. Tabs
retain repository identity and do not silently switch when the active editor
changes.

## Security and privacy

- Respect VS Code Workspace Trust before executing Compass or Git.
- Use strict webview Content Security Policy with nonce-bound local scripts and
  no network destinations.
- Bundle graph, diagram, icon, style, and font-independent assets locally.
- Validate every host/webview message against versioned runtime schemas.
- Render source-derived labels and metadata as text, not `innerHTML`.
- Treat node IDs as opaque strings and source paths as untrusted input.
- Resolve source navigation against the selected repository and require an
  explicit decision before opening an unexpected external path.
- Store historical exports in extension-managed private storage with bounded
  size and lifecycle cleanup.
- Do not log graph payloads, query results, environment credentials, provider
  tokens, or source contents by default.
- Do not add telemetry in the first release.

## Error handling

Every primary view has explicit loading, empty, stale, unsupported, failed, and
cancelled states. Errors retain repository, command, commit, and realization
context without exposing secrets.

Required behaviors include:

- missing CLI: guided installation and path selection;
- incompatible CLI: required/installed version explanation;
- no Compass project: guided initialization;
- missing current graph: update action;
- stale current graph: view with warning and update/watch actions;
- malformed public artifact: diagnostic and rebuild action;
- unavailable history store: enable or build guidance without implicit writes;
- missing historical realization: explicit build action;
- corrupt or invalid realization: no rendering or fallback substitution;
- incompatible fingerprints: comparison disabled with rebuild guidance;
- partial call coverage: visible limitations and evidence;
- truncated call expansion or graph overview: visible bounds and continuation;
- failed watch process: retained logs and restart action; and
- lost remote connection: operations marked disconnected until process state is
  re-established.

## Performance

- Open graphs above 5,000 nodes in community overview mode.
- Load exact node neighborhoods on search or focus.
- Expand symbol call graphs lazily with explicit node, edge, and depth limits.
- Virtualize the all-commit timeline and result tables.
- Move layout and expensive transformation work to web workers where profiling
  shows main-thread blocking.
- Retain no more than three decoded historical graphs.
- Stream historical exports to private files instead of retaining duplicate
  full payloads in extension-host memory.
- Cancel obsolete requests when selection changes.
- Preserve stable positions across compatible graph replacements.

Performance gates use representative small, medium, and large fixture
repositories and measure activation, first useful render, graph interaction,
timeline scrolling, call expansion, memory, and cancellation latency.

## Accessibility

The extension targets WCAG 2.2 AA within webview constraints:

- full keyboard access to graph controls, search results, inspectors, timeline,
  call expansion, and query results;
- visible VS Code-compatible focus treatment;
- status and error live regions;
- non-color indicators for selection, call resolution, graph state, and diff;
- minimum 44-by-44 CSS pixel touch targets in narrow layouts;
- reduced-motion behavior for layout focus and transitions;
- high-contrast theme support;
- meaningful accessible names for icon buttons; and
- source and graph alternatives for information that cannot be conveyed
  reliably by the canvas alone.

## Testing strategy

### Rust and schema tests

- capability, timeline, call-graph, call-flow, and progress-event schema
  contracts;
- inspection-only guarantees for timeline and status;
- history identity and exact-revision export;
- Program IR coverage and unresolved-call behavior;
- offline HTML assembly and asset-manifest validation;
- escaping, CSP, atomic publication, and no-runtime-network guarantees; and
- compatibility tests for existing graph, query, diff, and history outputs.

### TypeScript unit tests

- CLI discovery and capability negotiation;
- command argument construction without shell parsing;
- process lifecycle, progress parsing, cancellation, and concurrency;
- workspace/repository selection and tab identity;
- runtime schema validation and additive-field compatibility;
- source path and location handling;
- historical cache eviction and temporary-file cleanup; and
- graph, call-graph, history, query, and diff state reducers.

### Component and browser tests

- graph stabilization, focus, spotlight, search, filtering, and inspector;
- export/webview interaction parity using the same behavioral fixtures;
- caller/callee expansion and resolution-state presentation;
- architecture-flow navigation and tables;
- timeline virtualization, selection, missing-state actions, and merge parents;
- stable cross-revision positions and comparison overlays;
- keyboard, screen-reader semantics, reduced motion, responsive layout, and all
  supported VS Code themes;
- corrupted revision isolation and stale-graph behavior; and
- strict CSP and absence of external network requests.

### VS Code integration and packaging tests

- activation with and without Compass installed;
- trusted and untrusted workspaces;
- init, update, watch, query, history build, cancellation, and diff against
  fixture repositories;
- source navigation from graphs and results;
- multi-root repository selection;
- tab restoration;
- Remote SSH/WSL/Dev Container extension-host assumptions;
- VSIX install and activation smoke tests on macOS, Linux, and Windows; and
- Marketplace manifest, icon, license, and packaged-asset validation.

## Implementation milestones

### Milestone 1: Foundation and current graph

- Establish the shared viewer workspace and asset pipeline.
- Refactor current `graph.html` to the shared viewer without losing behavior.
- Add the VS Code extension shell, branding, themes, setup, handshake,
  repository sessions, output, and process manager.
- Add guided `init`, `update`, and `watch`.
- Ship current graph exploration and source navigation.

### Milestone 2: Calls and queries

- Add structured symbol call-graph and architecture-flow contracts.
- Ship cursor resolution, lazy caller/callee expansion, evidence, and source
  navigation.
- Move `callflow-html` presentation to the shared architecture-flow viewer.
- Ship natural-language and CompassQL views.

### Milestone 3: Evolution

- Add the inspection-only all-commit timeline contract.
- Ship timeline states, explicit materialization, progress, retry, and
  historical graph loading.
- Add bounded historical caching, stable layout transfer, parent selection,
  graph comparison, semantic diff, and finding evidence.

### Milestone 4: Release qualification

- Complete cross-platform, remote-host, accessibility, security, compatibility,
  and large-graph gates.
- Produce and smoke-test the Marketplace VSIX.
- Update Compass command, output, setup, and extension documentation.

No milestone is the complete release by itself. Version 1 is accepted only when
all four milestones pass their required gates.

## Acceptance criteria

The first complete release is acceptable when:

1. A new user with a supported installed Compass CLI can initialize a project,
   build or watch its graph, and open the current graph without using a shell.
2. The exported current graph and VS Code graph use the same shared interaction
   implementation and pass common behavioral tests.
3. A user can start at the active symbol, expand callers and callees, understand
   resolution and coverage limits, and navigate to evidence.
4. A user can open the architecture call-flow document without runtime network
   dependencies.
5. A user can run natural-language and CompassQL queries against current or
   selected historical graphs and navigate structured results.
6. The History view shows every Git commit and accurately distinguishes all
   required materialization states without triggering implicit builds.
7. A user can explicitly build a missing revision, observe progress, open its
   exact graph, compare it with a chosen parent, and inspect semantic findings.
8. Multi-root, untrusted, stale, partial, corrupt, incompatible, cancelled, and
   remote-disconnected states behave as specified.
9. The VSIX passes supported platform, security, accessibility, performance,
   and packaging gates.

