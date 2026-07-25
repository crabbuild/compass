# VS Code node source navigation

**Date:** 2026-07-25

## Goal

Let a user double-click a node in the Compass code graph, call graph, or
historical graph to open its source file at the exact recorded location.
Single-click continues to focus the node and populate the inspector.

## Interaction

- A single click selects and focuses the node.
- A double-click opens source only when the node has a non-empty file plus
  either a line or byte position.
- A node without a complete source location remains selectable and does
  nothing on double-click.
- The existing **Open source** inspector button remains available for
  discoverability and keyboard access.
- Search results and connected-node controls continue to focus nodes without
  opening an editor.

This avoids disruptive editor navigation during ordinary graph exploration
while making source access direct and predictable.

## Architecture

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

## Location rules

A source is navigable when:

- `file` is not empty; and
- at least one of `startLine`, `endLine`, `startByte`, or `endByte` is present.

Byte positions take precedence when available. Otherwise, navigation uses
one-based line positions and converts them to VS Code's zero-based positions.
Existing range and path-safety behavior remains unchanged.

## Error handling

Nodes without a navigable location produce no message and no error. Errors
from a present but invalid location—such as a missing file, repository
mismatch, or path escape—continue through the existing VS Code error message
path.

## Verification

Implementation verification will cover:

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
