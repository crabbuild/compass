# Compass Pierre Diffs bundle

`pierre-diffs-v1.2.12.js` is a modified browser bundle of
`@pierre/diffs` 1.2.12. Compass exposes only `FileDiff` and
`parsePatchFiles`, supplies a Compass-specific dark theme, and limits Shiki to
its plain-text language. This keeps generated semantic-diff reports
self-contained without shipping every syntax grammar.

The bundle is distributed under the Apache-2.0 license in
`../pierre-diffs-LICENSE.md`.

To rebuild it with Bun:

```bash
bun install
bun run build
```
