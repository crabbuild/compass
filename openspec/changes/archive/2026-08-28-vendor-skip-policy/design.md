# Design: vendor discovery policy

## Decision

Leave `vendor/` out of the built-in skip list.

## Rationale

- **Skip** would silently remove real, build-relevant Go source and change graph
  identity for existing users.
- **Skip unless workspace member** is ecosystem-specific: Rust workspace
  membership does not describe Go vendor packages or other valid vendored source,
  and generic discovery must not depend on parsing one build system first.
- **Leave and allow explicit exclusion** preserves compatibility and uses the
  existing deterministic scope controls: `.compassignore`, saved project scope,
  and `--exclude`.

The behavior applies equally to initial discovery and filesystem watchers. This
change records and tests the policy; it does not introduce a new heuristic.
