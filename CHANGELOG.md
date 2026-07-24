# Changelog

## Unreleased

- Correct C function identities by resolving the callable declarator before
  generic declaration names, including macro-heavy SQLite declarations.
- Preserve repeated Markdown sections and rationale entries as distinct
  positional graph nodes.
- Advance the AST extraction cache namespace to `v0.9.21`. The first update
  after upgrading refreshes deterministic AST facts, then unchanged updates
  reuse the new cache normally.
- Add a development-only Graphify superset comparator and guarded Podman
  qualification script for node, edge, cold, warm, and query measurements.
