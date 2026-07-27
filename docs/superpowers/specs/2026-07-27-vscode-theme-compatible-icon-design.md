# VS Code Theme-Compatible Icon Design

## Context

Compass Codegraph `0.1.2` uses a transparent navy Marketplace icon. The mark
has adequate contrast on a light page but nearly disappears against the Visual
Studio Marketplace dark theme. The Marketplace accepts one static PNG and does
not select separate light and dark extension icons.

VS Code's Activity Bar has a different asset contract. It expects a
single-color SVG that the workbench can color for the active light, dark, or
high-contrast theme.

## Goals

- Keep the supplied compass geometry recognizable.
- Give the Marketplace icon dependable contrast on light and dark surfaces.
- Let the Activity Bar icon follow VS Code theme colors, including
  high-contrast themes.
- Make the intended use of each asset explicit and remove unused theme
  variants.
- Release the change as extension version `0.1.6` in a separate pull request.

## Asset Strategy

### Marketplace

`media/icon.svg` is the canonical 128-by-128 source. It uses a fully opaque,
rounded indigo tile with a white compass mark and a subtle inset keyline. The
tile supplies its own background, so the logo never depends on the surrounding
Marketplace theme for contrast. The `#5865F2` tile has a 4.61:1 contrast ratio
against white and a 3.62:1 ratio against VS Code's common `#1E1E1E` dark
surface; the white mark has a 4.61:1 ratio against the tile.

`media/icon.png` is a deterministic 128-by-128 render of that SVG. The central
tile behind the mark must be fully opaque; only the decorative pixels outside
its rounded corners may be transparent. The PNG must be included in the VSIX
and visually inspected against both near-white and near-black backgrounds.

### VS Code workbench

`media/compass.svg` remains a simple 24-by-24 monochrome silhouette. It uses
`currentColor` and contains no branded background, gradient, or fixed
light/dark color. VS Code can therefore apply the active foreground treatment
when the icon appears in the Activity Bar.

The unused `compass-light.svg` and `compass-dark.svg` files are removed. They
are not selected by the `viewsContainers.activitybar` contribution and imply a
theme-switching behavior that does not exist.

Command icons continue to use VS Code Codicons, which already follow the active
theme.

## Release Changes

- Bump `editors/vscode/package.json` from `0.1.2` to `0.1.6`.
- Update the workspace lockfile entry to `0.1.6`.
- Add a `0.1.6` changelog entry describing the cross-theme icon treatment.
- Keep the extension name, publisher, display name, commands, and settings
  unchanged.

## Verification

- Parse both retained SVG files as XML.
- Confirm the Marketplace PNG is exactly 128 by 128 and the mark sits entirely
  on the opaque tile.
- Render a light/dark comparison image for visual inspection.
- Run the VS Code extension typecheck and test suite.
- Build and smoke-test `crabbuild-compass-vscode-0.1.6.vsix`.
- Verify the packaged manifest version and packaged icon.
- Run `git diff --check`.

The Graphify codegraph will not be refreshed, following the project owner's
explicit direction for this work.
