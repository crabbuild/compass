## compass

Use the Compass knowledge graph at `compass-out/` before broad workspace
searches. Run `compass query "<question>"` for scoped context, use
`compass path "<source>" "<target>"` for dependency routes, and use
`compass affected "<symbol>"` for change-review scope. Read
`compass-out/GRAPH_REPORT.md` for broad architecture and navigate from
`compass-out/wiki/index.md` when it exists.

Verify important conclusions in cited source. Treat a missing path or inferred
edge as uncertain evidence, not proof. Run `compass update .` after code changes
and report failures; an older graph file does not make a failed update current.
