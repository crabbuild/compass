# VS Code History Diff Experience Design

## Context

The Codebase Evolution comparison view currently renders source patches as raw
preformatted text. The patches are exact but difficult to scan because they
lack syntax highlighting, line-number gutters, word-level emphasis, and useful
layout controls.

The changed graph already assigns added, removed, changed, and context states to
nodes and edges. However, the canvas, toolbar, inspector, and community filters
do not explain those states as one coherent comparison mode. Dense deltas can
look like an ordinary graph with a different palette, and labels compete with
each other before the user understands what changed.

Compass already vendors Diffs 1.2.12 for standalone semantic-diff HTML reports.
The VS Code viewer will align with that version and visual language, using
<https://diffs.com/docs> as the integration authority.

## Goals

The comparison experience should let a developer:

1. scan source edits using a familiar code-review diff;
2. switch between split and unified layouts without losing the exact patch;
3. understand graph-change states before interacting with the canvas;
4. filter the delta by change type;
5. inspect any node without relying on color alone; and
6. recover gracefully when an enhanced diff cannot render.

## Selected architecture

The work remains in the shared React viewer. It adds the React entry point from
`@pierre/diffs` at the same 1.2.12 version already used by the Compass CLI.

`SemanticFindings` delegates each validated source change to a focused source
diff component. That component owns Diffs rendering, responsive layout,
collapse state, and the exact-patch fallback. The semantic findings list keeps
its existing responsibility for non-source findings.

The graph remains the existing `CompassGraph`. Comparison mode is derived from
node or edge `change` metadata, so no Rust command, JSON schema version, or host
message changes are required. Comparison-specific state is limited to visible
change types and display preferences.

## Source changes

### File organization

The Source changes section gains a compact toolbar:

- Split and Unified layout choices;
- a Wrap lines toggle;
- Expand all and Collapse all actions;
- the number of changed files.

Each file is presented as a bordered review card with its path, normalized
status, and Diffs-provided addition/deletion statistics. The first file opens
by default; later files remain collapsed until requested. This makes the first
change immediately useful without turning a long comparison into one
unstructured page.

### Diffs rendering

Each expanded patch is rendered through the high-level React diff API. The
renderer uses:

- a split layout on panels wider than 760 pixels;
- a unified layout at 760 pixels or below;
- the user's explicit Split or Unified preference on wider panels;
- classic addition and deletion indicators;
- word-level changed-token emphasis;
- metadata-style hunk separators;
- line numbers;
- horizontal scrolling by default, with optional wrapping.

The effective layout updates when the webview width crosses the breakpoint.
Split is disabled while the panel is too narrow to present both sides legibly.
The selected wide-screen layout and wrapping preference live for the mounted
history tab.

Expanded files render lazily. Collapsing a file removes its expensive diff body
while retaining the exact patch data and header. The integration does not add a
worker pool initially because VS Code webview workers require extra resource
and content-security wiring; lazy file rendering bounds the initial work.

### Theme

Diffs selects its bundled light or dark syntax theme from the VS Code webview
theme. Supported Diffs custom properties map the surrounding surface, gutter,
line-number, addition, deletion, and modified colors to VS Code editor and Git
decoration tokens. Shadow-DOM overrides stay small and target documented data
attributes only.

High-contrast themes add explicit borders and preserve line indicators even
when background colors are unavailable.

### Failure handling

The report is treated as untrusted structured data. A source change is
renderable only when its path and patch fields are valid strings.

If Diffs fails to parse, highlight, or mount a patch, that file card shows:

> Enhanced diff unavailable. Showing the exact Git patch.

The raw patch then appears unchanged in a scrollable preformatted block. One
bad file does not prevent other files, semantic findings, or the graph from
rendering.

## Graph comparison

### Comparison summary

The current compact number strings become readable Node and Edge summaries.
Each states explicit Added, Removed, and Changed counts. Empty categories remain
visible as zero only in the summary; the canvas legend omits categories with no
visible records.

The comparison heading continues to identify both revisions and retain the Exit
comparison action.

### Change legend and filters

The graph stage receives a persistent comparison legend with four textual,
toggleable controls:

- Added;
- Removed;
- Changed;
- Context.

Each control contains a marker, label, and count. All are enabled initially.
Disabling a type hides nodes of that type and any edge whose endpoint becomes
hidden. Community visibility and change-type visibility compose rather than
overwriting one another.

In comparison mode, Change types becomes the primary inspector filter.
Communities move into a collapsed secondary section because change status is
the first question in this workflow. Outside comparison mode the inspector
remains unchanged.

### Canvas treatment

Change state takes precedence over ordinary community color:

- Added uses the VS Code Git added-resource color.
- Removed uses the Git deleted-resource color.
- Changed uses the Git modified-resource color.
- Context uses the description foreground with reduced opacity.

Added, removed, and changed nodes are slightly larger than context nodes.
Context nodes remain available to explain relationships but recede visually.

Edge treatment prioritizes change state over confidence:

- added edges are solid green;
- removed edges are dashed red;
- changed edges are emphasized amber;
- context edges remain quiet gray.

Confidence remains available in the edge tooltip. The comparison legend and
inspector provide textual meaning, so color is never the only status signal.

Automatic labels are limited to the highest-degree changed nodes, up to twelve,
plus the focused node. Context labels stay hidden unless focused. The existing
Show labels action still reveals every label.

### Node inspection

The hover card and pinned inspector show an Added, Removed, Changed, or Context
badge before ordinary node metadata. The badge uses text and a status marker.
Source navigation, neighbors, community identity, and signatures continue to
work as they do in the ordinary graph.

## Data flow

```text
semantic diff JSON
  -> validate source_changes
  -> per-file Diffs renderer
  -> raw patch fallback on failure

parent + current graph models
  -> compareGraphs
  -> nodes/edges with change metadata
  -> comparison-aware CompassGraph
  -> change filters + canvas + inspector badges
```

No comparison preference or diff content leaves the webview.

## Verification

Component and browser coverage will verify:

- valid patches render through Diffs rather than a raw `<pre>`;
- split, unified, and wrapping controls update mounted file diffs;
- narrow panels force a usable unified layout;
- collapsed files do not mount their diff body;
- malformed or failed patches show the exact fallback;
- comparison summaries use explicit labels and correct counts;
- change filters hide the expected nodes and connected edges;
- status colors follow VS Code tokens and high-contrast borders;
- hover and inspector surfaces expose textual change status;
- comparison-mode label selection remains bounded;
- ordinary non-comparison graphs retain their existing behavior;
- viewer tests, browser tests, VS Code builds, and VSIX smoke checks pass.

After code changes, `graphify update .` refreshes the project graph.

## Scope boundaries

This design does not add source editing, patch application, comments,
side-by-side node-property comparison, semantic-diff generation, new Rust
protocols, or a persistent user setting. It improves the presentation and
interaction of evidence Compass already produces.
