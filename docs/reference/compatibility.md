# Compatibility and evolution

Compass is an independent native product. Its public behavior is defined by
Compass documentation, native tests, and versioned Compass formats. It has no
Graphify runtime or test dependency.

> **Who this reference is for:** users, integrators, and contributors planning
> upgrades or relying on a Compass interface.

## Product contract

The supported identity is:

| Surface | Contract |
| --- | --- |
| executable | `compass` |
| artifact root | `compass-out/` |
| ignore file | `.compassignore` |
| project configuration | `.compass/` |
| environment | `COMPASS_*` |
| MCP | server `compass`, resources `compass://...` |

There is no alternate legacy frontend or alias. Legacy names and stores are
not read as fallback inputs.

## Native evidence

Changes are checked by native unit, integration, CLI, protocol, format,
cross-platform, fuzz, sanitizer, and mutation tests. The root
[`COMPATIBILITY.md`](../../COMPATIBILITY.md) lists the standard verification
commands.

## Intentional hard cutovers

Compass is early in its lifecycle and currently prefers a clean contract over
backward-compatibility branches. When a format or schema is cut over:

1. old data must be archived or removed;
2. a fresh Compass build creates current artifacts;
3. unknown major versions are rejected;
4. migration documentation states the exact operator action.

See [`MIGRATION.md`](../../MIGRATION.md) for the current cutovers.

## Project lineage

Compass was inspired by
[Graphify](https://github.com/Graphify-Labs/graphify). The projects now evolve
independently; that attribution does not make Graphify part of Compass.
