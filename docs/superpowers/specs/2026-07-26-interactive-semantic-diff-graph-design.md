# Interactive semantic-diff graph design

**Date:** 2026-07-26
**Status:** Approved for implementation planning

## Problem

The standalone `compass diff --format html` report renders its changed graph as
a bounded custom SVG. It is deterministic and self-contained, but dense reports
produce overlapping labels, weak hierarchy, and no selection behavior. The
result communicates that graph changes exist without helping a reviewer inspect
an individual changed node or understand its immediate impact.

The VS Code Codebase Evolution graph is outside this design. It already uses the
shared interactive `CompassGraph` component. This work applies only to the
standalone offline semantic-diff HTML report.

## Goals

- Make the sampled changed subgraph readable at a glance.
- Let mouse and keyboard users select a node.
- Keep the selected node and its immediate neighborhood visually prominent.
- Show accurate node, relationship, source, and semantic-finding details in a
  persistent inspector.
- Preserve deterministic output, offline operation, strict CSP compatibility,
  bounded rendering, and exhaustive list fallbacks.
- Remain useful on narrow screens and with reduced motion or high contrast.

## Non-goals

- Replacing the standalone renderer with the React/vis-network Compass viewer.
- Changing the semantic-diff JSON schema.
- Adding old and new attribute values that the current `GraphNodeDelta` and
  `GraphEdgeDelta` records do not retain.
- Rendering every changed node or edge in the visual sample.
- Adding remote assets, hosted services, or runtime dependencies.
- Changing the VS Code comparison graph.

## Chosen approach

Enhance the existing deterministic SVG as a **change lens**. The report keeps a
bounded topology view, but nodes become readable capsules with collision-aware
placement. Selecting a node opens a persistent inspector and emphasizes its
direct neighborhood while dimming unrelated topology.

Embedding the full Compass viewer was rejected because it would add a large
runtime and duplicate application infrastructure inside every offline report.
A status-column dependency map was rejected because it makes change categories
clear but weakens arbitrary topology exploration.

## Layout

Desktop uses a graph explorer split:

```text
┌──────────────────────────────────────────────────────────────┐
│ Graph summary and change legend                              │
├──────────────────────────────────────┬───────────────────────┤
│ Interactive changed-subgraph        │ Selected node inspector│
│                                     │                        │
│ readable node capsules              │ identity and status    │
│ directional relationship lines      │ kind and source file   │
│ selected neighborhood focus         │ changed field names    │
│                                     │ incoming relationships │
│                                     │ outgoing relationships │
│                                     │ related findings       │
│                                     │ source-patch link      │
├──────────────────────────────────────┴───────────────────────┤
│ Exhaustive node and edge lists                               │
└──────────────────────────────────────────────────────────────┘
```

The graph occupies approximately 70% of the available width and the inspector
30%, with a practical minimum inspector width near 280 pixels. Below 760 pixels,
the inspector moves below the graph and uses the full width.

The inspector starts with a short invitation to select a node. It does not
preselect a node or dim the graph before the reviewer acts.

## Visual system

The graph derives all styling from the report's existing palette:

| Role | Token |
| --- | --- |
| Canvas | `--surface-inset` (`#0b0e13`) |
| Node surface | `--surface-raised` (`#191e27`) |
| Selection and keyboard focus | `--accent` (`#8ab4f8`) |
| Added | `--green` (`#65bd84`) |
| Removed | `--red` (`#ff7b86`) |
| Changed | `--amber` (`#d9a441`) |
| Context | `--muted` (`#8d96a5`) |

Node capsules use the UI font for the readable label and the monospace stack
for kind, path, and identifier details. Change state is encoded by border
style, a visible `+`, `−`, `~`, or `·` mark, and an accessible label; color is
not the only signal.

Directly changed and higher-degree nodes may receive slightly stronger visual
weight. Node size does not encode arbitrary magnitude. The renderer avoids
glow, gradients, and continuous physics animation. Selection and hover opacity
transitions last about 140 milliseconds and are disabled under
`prefers-reduced-motion`.

## Graph construction and sampling

The renderer creates one node index from:

- added, removed, and changed node deltas;
- source and target endpoints from added, removed, and changed edge deltas; and
- any available label, kind, source-file, and changed-field metadata.

An endpoint without a corresponding node delta is a context node. It displays
its identifier and known relationships without inventing kind or source data.

The visual sample remains bounded. Ranking proceeds in this order:

1. changed, removed, and added node deltas;
2. direct endpoints connected to those nodes, ordered by changed-edge degree;
3. remaining context endpoints in deterministic identifier order.

The exact node and edge caps remain implementation constants and must be covered
by tests. The note below the canvas reports sampled and exhaustive counts when
the visual is truncated.

Placement remains deterministic and topology-first. The layout uses capsule
dimensions in collision calculations so labels do not overlap neighboring
nodes. Edges render behind nodes with directional arrowheads. Labels truncate
inside capsules, while full labels remain available to accessibility APIs and
the inspector.

## Selection and neighborhood focus

A node can be selected by click, Enter, or Space. Selection:

- adds a strong accent focus ring to the selected capsule;
- retains full status styling for directly connected neighbors;
- brightens incoming and outgoing edges that touch the selection;
- dims unrelated nodes and edges to approximately 15–20% opacity; and
- updates the persistent inspector and an `aria-live` status.

Clicking another node or an inspector relationship selects that node. Clicking
empty canvas space or pressing Escape clears selection and restores the full
graph. Hover may preview emphasis but must not replace persistent selection.

## Inspector

The inspector shows only facts available in the report:

- change status;
- label and identifier;
- kind, when known;
- source file, when known;
- changed field names, when present;
- incoming changed relationships;
- outgoing changed relationships;
- related semantic findings; and
- a source-patch navigation link when the file has a matching source change.

Each relationship row includes direction, relation, neighbor label or
identifier, and edge change status. Selecting a relationship row moves focus to
the neighbor even when that neighbor is outside the visual sample. In that
case, the inspector remains useful and the canvas explains that the selected
node is outside the bounded visual sample.

Related findings are matched through finding subjects and evidence record keys.
Source navigation matches normalized node source paths against old and new
source-change paths. Links are omitted when no valid target exists.

The schema currently retains changed-field names but not before-and-after field
values. The inspector must not imply that those values are available.

## Exhaustive fallback

The existing node and edge delta lists remain authoritative and exhaustive.
They continue to render below the explorer. If JavaScript is unavailable or
fails, those lists and the embedded `compass.semantic_diff.report/1` payload
still expose the complete comparison.

Graph list rows may gain stable anchors so inspector links can reveal the
matching exhaustive record. This must not alter report data or require
JavaScript for the list itself.

## Accessibility

- SVG nodes expose button semantics, readable names, and keyboard focus.
- Enter and Space select; Escape clears.
- Focus styling remains visible in high-contrast environments.
- Change marks and text accompany semantic colors.
- Selection updates a polite `aria-live` status.
- Inspector headings and relationship groups use semantic HTML.
- Truncated visible labels retain complete accessible names.
- Reduced-motion users receive immediate state changes.
- The responsive inspector follows the graph in reading order.

## Failure behavior

- Missing node metadata renders an explicit unavailable value or omits the
  unsupported row.
- Empty relationship and finding groups say that no changed records are known.
- Missing source matches do not create dead links.
- A graph with no meaningful changes retains the existing empty state.
- A graph-rendering exception leaves the exhaustive lists intact and reports a
  concise canvas fallback message without exposing internal paths.
- Hostile labels, identifiers, fields, relations, and paths remain escaped in
  HTML and are assigned to dynamic DOM through text-only APIs.

## Testing

Follow test-driven development. Tests must fail for the missing interaction
before production changes are written.

Rust renderer tests cover:

- explorer, inspector, live-region, and fallback markup;
- embedded report safety and absence of external scripts or styles;
- deterministic anchors and escaped hostile content;
- empty and populated graph states; and
- source/finding navigation targets.

Pure interaction tests cover:

- deterministic ranking and bounded sampling;
- context-node construction;
- incoming and outgoing relationship classification;
- selection, neighborhood membership, and clear behavior;
- inspector models for added, removed, changed, and context nodes; and
- out-of-sample neighbor selection.

A browser-level fixture covers:

- pointer selection;
- keyboard selection and Escape;
- unrelated-node dimming;
- inspector updates;
- clickable neighbor navigation;
- source or finding navigation when present; and
- the narrow-screen inspector layout.

Existing semantic-diff CLI and full-workspace tests remain regression gates.

## Acceptance criteria

1. The generated HTML remains one self-contained file with no network
   dependency.
2. Node labels in the bounded graph do not overlap other node capsules at the
   supported fixture sizes.
3. Selecting a node reveals accurate details and emphasizes exactly its direct
   changed-edge neighborhood.
4. Inspector relationship rows can select both visible and out-of-sample
   neighbors.
5. Keyboard and pointer interactions produce equivalent selections.
6. The exhaustive graph-delta lists remain present and complete.
7. Missing metadata and partial evidence are represented honestly.
8. The report passes its strict CSP, renderer tests, browser fixture, and
   workspace verification.
