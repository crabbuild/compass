# VS Code Workbench Enhancements

Date: 2026-07-25

## Objective

Make Compass for VS Code feel complete and understandable from first load through
ongoing repository work. The extension should provide a polished graph-loading
experience, a flexible graph inspector, automatic CLI discovery without persistent
path noise, a useful Operations command center, and clear access to Git revision
graphs.

## Scope

This change covers five connected improvements:

1. Replace the graph tab's top-left loading sentence with a centered, themed
   loading experience.
2. Make the graph inspector collapsible and resizable from the right edge of the
   graph workspace.
3. Keep CLI discovery automatic and show CLI setup details only when discovery or
   compatibility fails.
4. Turn the Operations tree into a command center while retaining active-operation
   status.
5. Make Codebase Evolution discoverable from Repository and Operations so users
   can inspect, build, compare, and query Git revision graphs.

The implementation will reuse existing commands, graph hydration, and history
workflows. It will not introduce a separate dashboard, silently build historical
graphs, or change Compass CLI contracts.

All repository graph artifacts remain Compass artifacts under
`<repository>/compass-out/`. This feature does not create or use a
`graphify-out/` directory.

## Experience Design

### Loading

The graph webview renders its loading UI immediately before posting its `ready`
message. The screen centers a Compass mark within a small constellation of connected
nodes. Subtle node pulses and edge motion suggest that relationships are being
assembled. The copy reads:

- Eyebrow: `Compass graph`
- Primary status: `Mapping your codebase`
- Supporting status: `Reading graph · Arranging relationships · Preparing inspector`

The animation is ambient rather than a fake progress meter. It stops or becomes
effectively static when the operating system requests reduced motion.

The loader uses VS Code theme tokens with fallbacks: editor background, panel
background, focus blue, testing green, muted foreground, and panel border. It uses
the VS Code interface font for copy and the editor monospace font for repository
details or graph statistics.

If hydration fails, the same centered shell becomes an error state. It states what
failed and provides `Retry` and `Show Compass output` actions. Retry asks the host
to run hydration again; Show Compass output reveals the extension output channel.

### Graph inspector

On wide screens the graph workspace has three columns: graph stage, separator, and
inspector. The separator supports pointer dragging and keyboard resizing. Its
accessible role is `separator`, its orientation is vertical, and arrow keys adjust
the width in consistent increments.

The inspector width is constrained to a useful minimum and maximum. The header
contains a collapse control. When collapsed, the inspector becomes a narrow right
rail with an expand control and the graph stage takes the remaining width. Width
and collapsed state persist in VS Code webview state for that graph tab.

On narrow screens the existing stacked inspector layout remains authoritative; the
desktop resize separator is hidden so the interface does not create a fragile
horizontal interaction on small viewports.

### Repository

The Repository view prioritizes workspace state:

- Each repository is a collapsible row with its graph state.
- An available graph exposes `Open graph` and `Codebase evolution` child actions.
- A missing graph exposes `Initialize repository`.
- A building or failed graph keeps the corresponding status icon and directs the
  user toward the active operation or retry action.

When an executable is discovered and compatible, the CLI path does not occupy a
permanent Repository row. When discovery fails, or the binary is incompatible, a
single setup row explains the problem and opens binary selection/setup.

CLI resolution remains configured executable first, followed by every executable
named `compass` on `PATH`, including the remote extension host's environment.

### Operations

Operations becomes a native VS Code tree command center. It groups contextual
actions under:

- Build: Initialize repository, Update graph, Start/Stop watch
- Explore: Open graph, Call graph from cursor, Architecture flow, Query codebase
- History: Codebase evolution
- Active operations: current build and watch processes

Items call existing extension commands. This keeps repository picking,
compatibility checks, progress notifications, cancellation, output logging, and
error handling centralized. Commands that cannot currently run remain discoverable
with a contextual description where that is helpful; actions whose prerequisite is
fundamentally absent are omitted.

Active operations appear before command groups so current work is immediately
visible.

### Git and build history

`Codebase evolution` opens the existing history workspace. The timeline lists every
reachable Git commit and its graph state. From a selected commit, users can:

- Open an available graph.
- Explicitly build a missing graph using the configured, code-only, or reused
  profile.
- Compare the commit with a parent after both graphs are available.
- Query the selected revision.
- Inspect structural change counts.

Historical graphs remain opt-in. Merely opening history never builds missing
revisions.

## Architecture and data flow

The shared `@compass/viewer` package owns inspector layout because it owns
`CompassGraph` and `GraphInspector`. `CompassGraph` stores the transient inspector
layout and accepts optional initial layout values plus a change callback. The VS
Code graph webview supplies values from `getState()` and persists changes with
`setState()`. Offline exports retain sensible defaults without requiring a host.

The VS Code graph webview owns loading and error presentation. It sends `ready`,
receives the existing hydration message, and adds host messages for retrying
hydration and revealing output. `GraphPanel` continues to own CLI export and schema
validation.

The tree providers own presentation only. `StatusTree` derives repository/setup
items from discovery and session state. `OperationsTree` derives command groups and
active rows from session state. Extension command handlers remain the single
execution path.

Repository sessions remain the source of truth for graph, writer, and watch state.
Refresh events update both tree providers and the status bar.

## Accessibility and resilience

- Loading status uses a live region without repeatedly announcing decorative
  animation.
- Loader graphics are hidden from assistive technology.
- Inspector controls have explicit accessible names, visible focus states, and
  keyboard resizing.
- Separator values expose current, minimum, and maximum widths.
- High-contrast themes receive visible borders and controls.
- Reduced-motion mode removes ambient loader motion and nonessential transitions.
- Long paths and operation descriptions truncate visually while remaining
  available as tooltips.
- Failed hydration remains readable and recoverable without reopening the tab.

## Testing and verification

Automated coverage will include:

- Inspector layout clamping, collapsing, and keyboard resize behavior.
- Loading-to-graph and loading-to-error rendering.
- Retry and output host messages.
- CLI discovery precedence and missing/incompatible presentation.
- Repository child actions for available and missing graphs.
- Operations grouping, contextual labels, commands, and active processes.
- Registration of every command used by tree items.

Verification will run the VS Code extension's type check, unit tests, production
build, integration tests where supported, package creation, and VSIX smoke check.
Viewer accessibility/parity tests relevant to the graph inspector will also run.

## Non-goals

- Replacing native VS Code tree views with a custom sidebar webview.
- Bundling or downloading the Compass CLI.
- Automatically materializing Git revision graphs.
- Changing graph export schemas or history CLI contracts.
- Redesigning unrelated query, architecture, or call-graph workspaces.
