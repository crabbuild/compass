# VS Code graph canvas parity and source navigation

**Date:** 2026-07-25

## Goal

Give the Compass VS Code code graph the visual language and interaction quality
of the richer Compass HTML export design preserved in `html.rs`, while keeping
the canvas native to VS Code themes. Let a user double-click a node in the code
graph, call graph, or historical graph to open its source file at the exact
recorded location.

## Product boundary

The active HTML exporter and VS Code extension already render the same shared
React `CompassGraph`. The implementation will rewrite that shared component
and its `VisNetworkCanvas` rather than creating a VS Code-only copy or embedding
an exported HTML document in the webview.

This preserves one graph model, one layout implementation, and one interaction
surface across:

- `compass export html`;
- the VS Code current graph;
- VS Code historical graphs; and
- the call graph adapter.

The dormant `page` implementation in `crates/compass-output/src/html.rs` is the
design reference, not a second runtime to revive.

## Visual design

The shared React graph will adopt the reference export's principal visual
elements:

- a layered radial canvas with a subtle dotted texture;
- a floating translucent toolbar with layout status and compact Lucide actions;
- a dedicated inspector sidebar with Compass identity, search, node metadata,
  connected nodes, community controls, and graph statistics;
- hover cards containing symbol kind, language, source file, line range, and
  signature when present;
- node sizes derived from degree or aggregate member count;
- stronger focused-node glow and relationship spotlighting;
- visually distinct extracted, inferred, and ambiguous edges; and
- responsive behavior that moves the inspector below the canvas in narrow
  views.

The rewrite retains React, shadcn components, Tailwind CSS, Lucide icons, and
the existing local-only webview asset policy.

## Theme behavior

Inside VS Code, every semantic surface uses VS Code CSS variables first:

- editor colors for the canvas and text;
- sidebar colors for the inspector;
- menu and widget colors for floating panels and hover cards;
- input, button, focus, warning, and error variables for controls and evidence;
- editor and UI font variables for proportional and monospace text.

The dark Compass export palette from `html.rs` supplies fallbacks when VS Code
variables are unavailable. A standalone HTML export therefore retains the
recognizable Compass dark appearance, while a VS Code webview follows the
active light, dark, or high-contrast theme without a reload-specific fork.
Community colors remain stable across hosts unless accessibility contrast
requires an outline supplied by the host theme.

## Canvas behavior

`VisNetworkCanvas` will use the reference export's ForceAtlas2-based tuning,
continuous edge curves, drag optimization, stabilization, and explicit
pause/resume behavior. It will expose events and imperative controls but will
not own inspector state.

`CompassGraph` will own:

- selected node and relationship spotlight state;
- search and keyboard result selection;
- physics, label, fit, and reset controls;
- hover-card content;
- community visibility and select-all behavior;
- inspector metadata and connected-node navigation; and
- source-open eligibility.

The existing community overview produced for graphs above the node limit
remains supported. Community drill-down is not added in this change because
the versioned viewer model does not contain the full hidden member subgraphs.
Adding that payload would be a separate contract and performance decision.

## Source interaction

- A single click selects and focuses the node.
- A double-click opens source only when the node has a non-empty file plus
  either a line or byte position.
- A node without a complete source location remains selectable and does
  nothing on double-click.
- The existing **Open source** inspector button remains available for
  discoverability and keyboard access.
- Search results and connected-node controls continue to focus nodes without
  opening an editor.

`VisNetworkCanvas` owns the distinction between the graph library's `click`
and `doubleClick` events. It continues to emit `onFocus(nodeId)` for a click
and adds `onOpenSource(nodeId)` for a double-click.

`CompassGraph` resolves the node ID against its validated graph model. It calls
the existing `GraphHost.openSource` bridge only when the node has a usable
source location. Call graphs already adapt their call anchors into the shared
graph source-location shape, so they inherit the same behavior without a
second navigation implementation. Historical graphs also reuse
`CompassGraph`.

The VS Code host continues to validate the message, confirm the repository
identity, reject paths outside the repository, open the document in preview,
convert byte or line coordinates to a VS Code range, reveal it, and select it.
The export viewer keeps its existing host bridge and receives the same
double-click behavior when its host supports source navigation.

## Viewer model

The versioned graph model gains optional, backward-compatible presentation
metadata needed by the richer canvas:

- `language`;
- `signature`;
- `size`;
- aggregate member count; and
- any learning-state presentation already emitted by Rust but not yet declared
  in the TypeScript contract.

Rust remains authoritative for sanitization, source lines, node sizing, color,
degree, and aggregate state. The React layer does not parse raw graph
attributes. Older `/1` payloads remain valid because every new field is
optional.

Source is navigable when:

- `file` is not empty; and
- at least one of `startLine`, `endLine`, `startByte`, or `endByte` is present.

Byte positions take precedence when available. Otherwise, navigation uses
one-based line positions and converts them to VS Code's zero-based positions.
Existing range and path-safety behavior remains unchanged.

## Error handling

Nodes without navigable source produce no navigation message and no error.
Missing optional presentation metadata degrades to the node label, kind, and
degree. Errors from a present but invalid location—such as a missing file,
repository mismatch, or path escape—continue through the existing VS Code
error-message path.

The canvas must remain usable when physics fails to stabilize quickly. It stops
after the bounded stabilization budget and leaves manual pause/resume, fit, and
reset controls available.

## Verification

Implementation verification will cover:

- the Rust viewer model serializes the optional presentation metadata;
- old `/1` graph payloads still validate;
- the shared canvas uses the reference ForceAtlas2 and edge options;
- light, dark, and high-contrast VS Code tokens have valid fallbacks;
- toolbar, inspector, hover card, community controls, and responsive layout
  render correctly;
- a canvas double-click emits the selected node ID;
- a graph with file and line metadata calls `openSource`;
- a graph with file and byte metadata calls `openSource`;
- a graph with only a file, or no source, does not navigate;
- single-click still focuses without opening source;
- call-graph anchors use the shared behavior;
- TypeScript, viewer tests, extension tests, Chromium checks, real VS Code
  activation, Compass update, VSIX packaging, and VSIX smoke validation pass.

Tests will be added after the implementation, consistent with the approved
non-TDD delivery approach.
