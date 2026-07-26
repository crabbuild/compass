# VS Code Marketplace Identifier Rename

## Goal

Publish the first-party Compass VS Code extension without colliding with the
existing Marketplace extension named `compass-vscode`.

## Naming Contract

- Marketplace manifest name: `crabbuild-compass-vscode`
- Marketplace publisher: `crabbuild`
- Marketplace extension ID: `crabbuild.crabbuild-compass-vscode`
- User-facing display name: `Compass`

The rename changes only the distribution identifier. It does not change the
extension directory, command identifiers, settings keys, view identifiers, or
product branding.

## Repository Changes

Update the extension manifest and npm lockfile to use
`crabbuild-compass-vscode`. Replace npm workspace selectors that depend on the
old package name with the stable workspace path `editors/vscode`.

Release artifacts use the new package name:
`crabbuild-compass-vscode-<version>.vsix`.

Update the VSIX smoke check to derive the expected filename from the extension
manifest's current name and version. This prevents an older VSIX in the same
directory from being selected accidentally.

## Verification

Run the extension typecheck, unit tests, build, package command, and VSIX smoke
check. Inspect the generated archive name and manifest identity. After code
changes, run `graphify update .` from the parent Graphify repository.

## Non-Goals

- Renaming the `editors/vscode` directory
- Changing the display name from `Compass`
- Changing `compass.*` commands or settings
- Publishing to the Marketplace during this change
