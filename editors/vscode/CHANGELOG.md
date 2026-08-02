# Changelog

## 0.2.0

- Highlight complete source lines when opening a code-graph node, including the
  full final line, instead of selecting only the symbol's byte range.
- Show recorded relationship source lines on graph edges and open the exact
  call or wiring site when a located edge is double-clicked.
- Hard-require current Compass community-detail capabilities for the graph
  workflow instead of entering a partially compatible viewer.
- Speed large immutable graph snapshots with copy-on-write clones where the
  host filesystem supports them, with a portable copy fallback.
- Share improved graph lookup performance and standalone light/dark canvas
  theming with `compass export html`.

## 0.1.9

- Add first-run Compass CLI installation in a visible VS Code terminal on
  macOS, Linux, and Windows.
- Verify and activate installed or manually selected CLIs without reloading the
  editor.
- Replace the architecture relationship grid with a production-first,
  directional subsystem map and complete paged cross-subsystem call evidence.
- Arrange subsystem maps into low-noise call-flow lanes with bundled routes,
  persistent drag positioning, focus highlighting, and collapsible details.
- Use the complete `compass.viewer.callflow/1` contract exclusively for
  negotiation, export validation, production scoping, and complete call evidence.
- Load architecture exports up to 128 MiB while keeping ordinary Compass
  commands at the 8 MiB safety ceiling.

## 0.1.8

- Add first-run Compass CLI installation in a visible VS Code terminal on
  macOS, Linux, and Windows.
- Verify and activate installed or manually selected CLIs without reloading the
  editor.

## 0.1.6

- Place the Marketplace logo on a high-contrast indigo badge so it remains
  visible in light and dark themes.
- Keep the Activity Bar logo monochrome and controlled by the active VS Code
  theme.

## 0.1.2

- Adopt the new Compass Codegraph logo for the Marketplace and VS Code
  Activity Bar.

## 0.1.1

- Allow the Codebase Evolution diff renderer's generated grid-span styles so
  installed VSIX builds display every changed source line at its natural height.

## 0.1.0

- Guided setup for a separately installed Compass CLI.
- Current code graph with the active Compass HTML export palette and interaction
  model adapted to VS Code light, dark, and high-contrast themes.
- Single-click node inspection and double-click source navigation when Compass
  has an exact file plus line or byte location.
- Cursor-rooted caller/callee graph with evidence resolution.
- Architecture flow and natural-language/CompassQL editor tabs.
- Complete Git evolution timeline, explicit historical builds, exact revision
  graphs, parent comparison, and semantic findings.
- Capability gates and guided recovery for older or incompatible Compass CLI
  binaries.
