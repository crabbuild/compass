# Compose task-oriented context

`compass context` builds one local evidence packet for a concrete coding task.
It resolves the target before composing evidence, verifies source bytes against
the graph digest, and records omissions instead of filling gaps with guesses.

```bash
compass context explain 'crate::Parser::parse'
compass context modify 'node:stable-id' --format json
compass context test 'PaymentGateway.charge' --max-depth 4
```

The result schema is `compass.task-context/1`. Evidence is ordered by product
priority: declaration and verified source, exact callers and callees,
implementation/type evidence when available, related tests, transitive impact
for modify/debug/test intents, then reflection memory linked to the exact node
identity. Every embedded structural record retains query provenance.

Fuzzy search results are candidates only. If zero or multiple exact identities
match, the result contains `not_found` or `ambiguous`, the candidates, and a
`target_resolution` omission. Compass does not select the first result.

`work` uses `compass.task-context-profile/1` and reports query, node, edge,
verified-file, source-byte, memory-read, and response-byte counts. Query,
source, knowledge, and aggregate response bounds are explicit. Lower-priority
sections are omitted deterministically when the aggregate response bound is
reached. The `resultDigest` excludes only its own value and the observational
response-byte count, so equivalent semantic evidence has a stable identity.

The MCP tool `task_context` exposes the same domain result inside the existing
`compass.mcp.tool-result/1` transport envelope. MCP transport truncation remains
separate from domain `truncated` and omissions.

No embeddings, model credentials, runtime downloads, or network access are
required. Reflection memory is optional and accepted only when its
`source_nodes` contains the resolved node identity.

## Related pages

- [Explore a codebase](exploring-a-codebase.md)
- [Integrate Compass](integrating-compass.md)
- [Provenance](../concepts/provenance.md)

**Next step:** run `compass context explain` with a qualified symbol name, then
use the returned exact node ID for modification planning.
