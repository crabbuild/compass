# Compass AST Cache v1 Hard Cutover

**Date:** 2026-07-27

## Goal

Replace the historical Graphify-derived AST cache namespace
`compass-out/cache/ast/v0.9.21/e6` with a Compass-owned compatibility
namespace:

```text
compass-out/cache/ast/v1/e6
```

The `v1` segment identifies Compass AST cache compatibility. It is independent
of the Compass package release and advances only when extractor behavior makes
existing AST entries unsafe to reuse. The `e6` segment remains the binary cache
encoding version.

## Hard-cutover behavior

`Cache::open` recognizes `v1` as the only current AST namespace. Its existing
stale-version cleanup removes every other `v*` directory under
`compass-out/cache/ast/`, including `v0.9.21`.

Compass does not read, copy, migrate, or fall back to entries in the old
namespace. The first build after upgrading is therefore a cold AST-cache build.
Semantic and Program IR caches are outside this cutover and remain untouched.

## Code design

In `compass-files`, rename the internal default and state from extractor-version
terminology to AST-cache-version terminology:

- `AST_EXTRACTOR_VERSION` becomes `AST_CACHE_VERSION`;
- `Cache::extractor_version` becomes `Cache::ast_cache_version`;
- the public extractor-version override is removed so callers cannot create
  alternate AST namespaces.

The default compatibility value is the explicit string `"1"`. It must not use
`CARGO_PKG_VERSION`, because normal Compass releases must not discard compatible
AST caches.

Directory selection and stale-cache cleanup continue to use the same version
value, ensuring that the namespace Compass writes is also the only namespace it
preserves.

## Documentation

The extraction-pipeline documentation will describe `v1` as the Compass-owned
AST compatibility namespace and state that incompatible extractor changes
advance it. Historical implementation plans remain historical records and are
not rewritten.

## Verification

A focused contract test will first require `ast/v1/e6` and verify that opening
the cache deletes a populated `ast/v0.9.21` directory. The test must fail before
the production constant and names are changed, then pass afterward.

Verification consists of:

1. focused `compass-files` cache contract testing;
2. the complete `compass-files` test suite;
3. formatting and Clippy for `compass-files`;
4. `graphify update .` after code changes, as required by the parent project.
