# VS Code Adaptive Edge Labels and Graph Loading Design

**Date:** 2026-07-26  
**Status:** Approved

## Approved Interaction Revision

The final interaction removes zoom-driven disclosure. Edge labels appear only
while their edge is hovered, while their edge is clicked/focused, when they are
incident to the focused node, or when the user explicitly enables `Show
labels`. Zooming never reveals additional labels.

The final loader also removes the knowledge-graph constellation. Its sole
visual mark is the current tilted Compass logo from the extension media assets,
paired with the existing indeterminate progress line and truthful phase copy.
These decisions supersede later references to close-zoom labels or a graph
constellation in this original design record.

## Problem

The standalone Compass HTML graph renders relationship information directly on
the graph, including labels such as `contains [EXTRACTED]`, `calls [INFERRED]`,
and `2 cross-community edges [AGGREGATED]`. The VS Code graph receives the same
relation and confidence data, but currently exposes it only through an edge
tooltip. Users therefore cannot understand relationship types while reading
the topology.

Large graphs also spend meaningful time copying a stable snapshot and deriving
an overview. The current extension has a React loading state and large-graph
phase messages, but the webview root is empty until the JavaScript bundle
starts. Its compact progress mark also does not communicate graph work as
clearly as it could. During a slow first load, either interval can read as a
blank or stalled page.

## Goals

The enhanced graph must:

1. show the same relation and confidence vocabulary as the HTML export;
2. reveal edge labels adaptively without turning dense graphs into a wall of
   text;
3. preserve `AGGREGATED` as a first-class overview confidence value;
4. display a useful loading surface from the webview's first paint;
5. describe only real loading phases and never imply fake percentage progress;
6. remain fast for graphs near the configured node limit; and
7. respect VS Code themes, high contrast, narrow panels, keyboard access, and
   reduced-motion preferences.

## Chosen Approach

Compass will use vis-network's native canvas edge labels and update their
visibility in place as graph interaction state changes. Native labels stay
anchored to curved edges and avoid maintaining a second DOM layout over the
canvas.

The extension will retain its existing staged graph-loading protocol. The host
HTML will include a small static first-paint loader, which React replaces when
the webview bundle starts. The React loader will use a restrained animated
knowledge-graph motif around the Compass mark and will update from the real
snapshotting and overview-export phases already reported by the host.

This is preferred over custom DOM edge labels because DOM overlays would add
viewport projection, collision, pan, zoom, and cleanup work on every frame. It
is preferred over relationship details only in the inspector because users
would still be unable to read relationship semantics in context.

## Edge Label Content

A single formatter will produce the visible and hover label for every edge:

- a relation with confidence becomes `contains [EXTRACTED]`;
- an inferred relationship becomes `calls [INFERRED]`;
- an aggregate overview relationship becomes
  `2 cross-community edges [AGGREGATED]`;
- a relation without confidence remains the relation; and
- a confidence without a relation is rendered in brackets.

Confidence is displayed in uppercase to match the standalone export. Relation
text remains unchanged because aggregate relationships already carry their
count and because custom relation names are valid graph data.

The TypeScript graph contract will accept `aggregated` in addition to
`extracted`, `inferred`, and `ambiguous`. The Rust viewer-model projection will
preserve `AGGREGATED` instead of falling through to `inferred`. Existing `/1`
payloads remain valid because no field becomes required.

## Adaptive Visibility

The canvas will keep edge labels hidden in the default wide view. A label
becomes visible when any of these conditions apply:

- the pointer is over that edge;
- either endpoint is the focused node;
- the graph is zoomed beyond a close-reading threshold; or
- the user enables the existing `Show labels` control.

`Show labels` will govern both node and edge labels so the control's existing
generic wording remains accurate. Turning it off restores adaptive behavior
rather than hiding labels that are needed for a focused or hovered
relationship.

The close-reading threshold will be a named, tested constant rather than an
incidental number inside an event handler. Zoom and hover events will update
the existing edge `DataSet` in place. They will not recreate the Network, reset
physics, or lose the user's viewport. Hidden-community and comparison filters
continue to control whether an edge itself is rendered.

Labels use the VS Code editor foreground, a compact UI font, and a
theme-derived canvas background behind the text. A small stroke/background
separates text from an edge without adding a pill to every relationship.
Focused and hovered labels use the full editor foreground; labels revealed only
by close zoom use the muted foreground. Relationship confidence continues to be
communicated by the existing solid or dashed edge treatment as well as text.

## Event and State Ownership

`VisNetworkCanvas` owns transient canvas facts: hovered edge ID and current zoom
scale. `CompassGraph` continues to own focused node and the explicit label
toggle. No relationship-hover state is added to the inspector.

The network event adapter will expose edge hover, edge blur, and zoom
information through typed callbacks. Node hover cards remain unchanged. A zoom
or drag clears transient hover state to prevent a label from remaining pinned
after its edge moves away from the pointer.

Pure helpers will own:

- edge-label formatting; and
- the adaptive edge-label visibility decision.

Keeping these rules outside the React effect makes the behavior deterministic
and unit-testable without constructing a canvas.

## Loading Experience

### First paint

`GraphPanel` will place accessible static loading markup inside `#root` before
loading the webview script. This surface uses the same bundled stylesheet and
appears while the browser downloads and evaluates React and vis-network.
React's first render replaces it with `GraphLoadingState`; there is no separate
state machine and no duplicated host request.

The static copy is the generic `Mapping your codebase` state because graph size
and phase are not known in the webview before the host responds.

### React loading state

The signature visual is a small graph constellation centered on the Compass
mark. A few nodes and edges suggest topology becoming connected; one restrained
tracer moves through the network. The surrounding typography and actions stay
flat and VS Code-native so the animation is the single expressive element.

The default state reads:

- `Compass graph`;
- `Mapping your codebase`; and
- `Reading graph · Arranging relationships · Preparing inspector`.

For an uncached graph of at least the existing eight-megabyte threshold, the
host includes the graph size and one of two real phases:

1. `snapshotting`: `Securing snapshot` is active;
2. `exporting`: `Snapshot ready` is complete and `Building overview` is active.

`Opening explorer` remains pending until graph hydration replaces the loader.
The UI distinguishes completed, active, and pending steps with text color and a
small state marker, but it does not show a numeric percentage because the
underlying operations do not report one. Cached and prepared overviews continue
to open immediately without an artificial minimum loader duration.

### Errors and recovery

If loading fails, the constellation becomes a quiet error state and ambient
motion stops. The existing error message, `Retry`, and
`Show Compass output` actions remain. Retry resets stale graph state, restores
the loading presentation, and requests hydration again.

## Visual Direction

All surfaces derive from VS Code semantic variables. The loader introduces no
remote font, image, or fixed theme. Its compact palette is:

- editor background: `--vscode-editor-background`;
- widget surface: `--vscode-editorWidget-background`;
- primary trace: `--vscode-progressBar-background`;
- graph accent: `--vscode-symbolIcon-classForeground`;
- structural border: `--vscode-widget-border`; and
- text and muted text: the existing foreground aliases.

The constellation is deliberately geometric rather than decorative: its nodes,
edges, and tracer encode the graph being prepared. Radii stay restrained and
shadows remain subtle so the loading surface feels modern without competing
with the editor.

At narrow widths, the visual and copy remain centered with no horizontal
scroll. At high contrast, nodes, edges, the Compass mark, actions, and focus
states use contrast borders. Under `prefers-reduced-motion: reduce`, the tracer
and node pulses stop while the status text continues to update.

## Data Flow

```text
GraphPanel creates webview HTML
  -> static first-paint loader is visible
  -> graph webview bundle starts
  -> React replaces static loader
  -> webview sends ready
  -> host checks prepared and cached overviews
  -> large uncached graph reports snapshotting/exporting phases
  -> host publishes validated GraphViewModel
  -> CompassGraph replaces loader
  -> canvas formats edge labels and reveals them adaptively
```

The loading work remains outside React. The host owns snapshotting, CLI
execution, caching, cancellation, and errors. The webview owns presentation and
interaction only.

## Performance and Failure Boundaries

Adaptive updates mutate labels on the existing edge `DataSet`; they do not
rebuild the network. The visibility function is linear in the already-rendered
edge set when zoom, focus, hover, or the explicit toggle changes. These events
are user-paced, and graph size remains bounded by `compass.graphNodeLimit`.

Hovering an edge that has been filtered or hidden does not make it visible.
Malformed relation or confidence values are rejected by the existing schema.
Older payloads without confidence continue to render relation-only labels.

If the static loader's stylesheet is delayed, its semantic text still appears
as ordinary document content. If the webview script fails entirely, the user
does not see an empty root; the static status remains visible while diagnostics
are available through VS Code output and reload behavior.

## Accessibility

The loader uses `role="status"` with polite live updates. Decorative
constellation geometry is hidden from assistive technology. Error content uses
`role="alert"`, and recovery buttons retain accessible names and visible focus.

Canvas labels are supplemental visual information. Relationship data remains
available through edge tooltips and the graph's validated model, so adaptive
visibility does not remove information from non-visual interfaces. The
existing keyboard graph controls can enable all labels, fit or reset the graph,
and navigate the inspector.

## Verification

Implementation-first delivery will add targeted regression tests immediately
after each bounded production slice. It will not use a red-green TDD sequence,
per the approved delivery constraint.

Unit tests will cover:

- exact edge-label formatting for extracted, inferred, ambiguous, aggregated,
  missing-confidence, and missing-relation cases;
- adaptive visibility for hover, focused endpoints, close zoom, explicit
  labels, and the default wide view;
- `AGGREGATED` preservation in Rust viewer-model output;
- acceptance of aggregated confidence in the TypeScript contract;
- typed edge-hover, edge-blur, zoom, drag, and node-hover event behavior;
- loading phase copy and active/completed/pending step state; and
- static first-paint markup in generated webview HTML.

Browser tests will verify:

- labels are absent in the initial wide view;
- focusing a node reveals only its connected relationship labels;
- hovering an edge reveals its label;
- close zoom reveals relationship labels and zooming out hides them again;
- `Show labels` reveals node and relationship labels without recreating the
  Network;
- aggregate edges display `[AGGREGATED]`;
- the loader is visible before graph hydration;
- large-graph phases show graph size and truthful step state;
- retry and output actions still work;
- reduced motion stops ambient animation; and
- light, dark, high-contrast, and narrow layouts remain readable.

Completion verification includes the affected Rust tests, viewer unit tests,
VS Code extension tests and type checking, production webview build, targeted
Playwright suites, VSIX packaging or the repository's equivalent extension
smoke check, and `graphify update .`.

## Scope Boundaries

This work does not add edge editing, relationship filtering, label-collision
solving, numeric progress estimation, remote assets, a new graph renderer, or a
new graph schema version. It does not change the configured node limit or make
large graphs load eagerly. It improves the presentation of data and phases the
extension already owns.
