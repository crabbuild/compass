# VS Code Graph Inspector Source and Neighbor Design

## Context

The graph inspector currently presents source metadata and source navigation as
two separate controls. The metadata card shows the file path, while a full-width
button below it repeats the path and line range. This consumes vertical space,
splits one task across two controls, and makes the source card appear
non-interactive.

Connected-node rows currently use a colored left border to indicate community.
In a narrow inspector this resembles selection or validation state and creates a
heavy visual rail.

## Goal

Make source navigation a single compact, precise action and make connected-node
community identity readable without decorative borders.

The user should be able to:

1. recognize the source location immediately;
2. open the exact recorded source range from the Source metadata card;
3. understand each connected node's community from a quiet colored dot; and
4. use every action with mouse, keyboard, high-contrast themes, or reduced
   motion.

## Selected design

### Clickable Source metadata card

When the selected graph node has a navigable source:

- the wide Source metadata card becomes a button;
- the primary line contains the source path;
- the secondary line contains `Line N` or `Lines N–M`;
- a Lucide `ExternalLink` icon appears at the trailing edge;
- activating the card calls the existing `onOpenSource` callback with the
  existing `SourceLocation`;
- the tooltip and accessible label contain the complete path and exact line
  range even when the visible path is truncated.

The separate full-width Open source button is removed.

When a source file is recorded without a navigable range, the Source card
remains non-interactive and shows that file path. It shows `Not recorded` only
when the node has no source file. It does not render a disabled button.

### Connected-node rows

Each connected-node row:

- removes the colored left border;
- adds an 8-pixel circular dot before the label;
- uses the neighbor's explicit background color when present;
- otherwise uses the neighbor community's color;
- falls back to the standard border color when no graph color exists;
- keeps the node label on one line with ellipsis;
- uses a subtle full-row hover and focus surface;
- retains the existing click behavior that focuses the connected node.

The dot is decorative because the community relationship is already available
through graph content and color alone must not carry action state.

## Visual system

The design continues to inherit VS Code theme tokens. It does not introduce
fixed product colors or new typography.

- Source card: existing metadata surface, slightly stronger hover/focus border.
- Source icon: `--compass-focus` at rest, foreground on hover.
- Neighbor dot: graph/community color.
- Neighbor row: transparent at rest, `--compass-focus-soft` on hover/focus.

The distinguishing gesture is consolidation: one source surface contains both
location evidence and the navigation action.

## Component boundaries

`GraphInspector` remains responsible for deriving:

- the navigable `SourceLocation`;
- the display line range;
- the community color for each neighbor.

No host protocol changes are required. The component continues to call the
existing `onOpenSource(source)` and `onFocus(nodeId)` callbacks.

Styling remains in the shared viewer theme so the web export and VS Code
webview retain visual parity.

## Accessibility

- The clickable Source card is a native `button`.
- Its accessible name uses an action phrase: `Open <path> at line N` or
  `Open <path> at lines N–M`.
- The full location is available through `title`.
- The ExternalLink icon and community dots are `aria-hidden`.
- Existing visible keyboard focus treatment is extended to the source card.
- High-contrast mode receives the same explicit border treatment as other
  inspector actions.
- No new animation is introduced.

## Error handling

The inspector only renders the interactive Source card when
`navigableSource(selected)` succeeds. Source-opening failures remain handled by
the existing VS Code host navigation path.

Long source paths and node labels truncate visually without losing their full
tooltip or accessible name.

## Verification

Post-implementation coverage will verify:

- navigable source renders one interactive Source card and no duplicate Open
  source button;
- the card exposes the exact path and line range;
- activation sends the unchanged `SourceLocation`;
- file-only nodes render their recorded path without a Source button;
- nodes without source metadata render a static `Not recorded` card;
- connected-node rows render dots and no colored left border;
- keyboard focus and serious accessibility checks remain clean;
- viewer and VS Code packages still build and the packaged VSIX passes smoke
  validation.

## Scope boundaries

This design does not change:

- graph contracts or source-range calculation;
- VS Code source-navigation behavior;
- graph canvas node styling;
- loading-screen design;
- Repository or Operations tree design;
- runtime output paths or Compass CLI behavior.

Loading and tree-view improvements will be designed and implemented as separate
follow-up deliverables so their state models and acceptance criteria remain
clear.
