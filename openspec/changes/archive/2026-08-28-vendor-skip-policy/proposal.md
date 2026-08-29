# Proposal: retain explicit vendor exclusions

## Why

The name `vendor/` does not reliably mean generated or disposable content.
Go repositories keep build-relevant source there, and Rust workspaces may make
a vendored package an explicit member.

## What Changes

- Deliberately retain the existing policy: `vendor/` is discovered by default.
- Document explicit `vendor/**` exclusion through project scope or
  `.compassignore` for repositories that do not want that source indexed.
- Pin build and watcher behavior for both a workspace member and Go vendor source.

## Compatibility

This decision preserves current discovery scope. No migration action is needed.
