# Compatibility and evolution

Compass is an independent native product. Its public behavior is defined by
Compass documentation, native tests, and versioned Compass formats. It has no
Graphify runtime or test dependency.

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

Both stdio and Streamable HTTP use only MCP 2026-07-28 and reject older
revisions. HTTP is stateless and has no MCP-2025 session fallback. See the root
compatibility and migration documents for the `--session-timeout` 0.4.x
deprecation and 0.5.0 removal schedule.

The four core navigation tools return `compass.code_context.v1` structured
content and advertise matching output schemas. Their former
`compass.query/1` response is preserved under `data`; clients that consumed the
old top-level shape must follow the root migration guide. MCP `resultType` is
the separate protocol discriminator. Remaining text-only results are marked
deprecated from 0.4.0 in discovery but remain callable; removal is not
scheduled before typed replacements ship.

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
