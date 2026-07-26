# VS Code Query Tab Indicator Design

## Context

The Ask Codebase workspace exposes two semantic tabs: Ask the codebase and
CompassQL. Their `role="tab"`, `aria-selected`, and panel associations are
correct, but the active styling is visually lost against the query composer.
The existing bottom border sits directly against another horizontal boundary,
while the active and inactive surfaces are nearly identical.

## Goal

Make the selected query mode immediately recognizable without making the header
feel heavier than the surrounding VS Code workbench.

## Selected design

The query-mode rail keeps its current two-column layout and content. The active
tab receives:

- a 2-pixel top accent using `--vscode-tab-activeBorder`, falling back to the
  existing query focus color;
- the VS Code active-tab background and foreground, with the existing raised
  query surface as the fallback;
- full-opacity icon and title treatment;
- a quiet inset boundary that separates it from adjacent inactive tabs.

Inactive tabs remain transparent and muted. Hover uses the existing workbench
hover surface and does not resemble the selected state.

The former active bottom border is removed. The rail's shared bottom border
continues to connect the tab strip to the composer panel.

## Interaction and accessibility

No component contract or selection behavior changes. The existing button-based
tabs continue to expose:

- `role="tab"`;
- `aria-selected`;
- `aria-controls`;
- the matching tab panel's `aria-labelledby`.

Keyboard focus remains a separate 2-pixel focus ring so focus and selection are
not conflated. High-contrast themes use `--vscode-contrastBorder` around the
active tab in addition to the top indicator. The design introduces no motion.

## Responsive behavior

The indicator remains visible at every supported width. Narrow layouts retain
the two equal-width tabs, truncate descriptions before labels, and do not
convert the controls into a menu or dropdown.

## Verification

Browser coverage will verify:

- Ask the codebase starts selected and exposes the top accent;
- switching to CompassQL transfers the active surface and indicator;
- the inactive tab does not retain selected styling;
- tab semantics and keyboard focus remain intact;
- dark, light, and high-contrast token fallbacks remain readable;
- the shared viewer and VS Code extension still build.

## Scope boundaries

This change does not alter query execution, query state, result rendering,
copy, tab order, or host messages.
