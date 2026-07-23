## compass

This project has a Compass knowledge graph at `compass-out/`.

Rules:

- Run `compass query "<question>"` before broad source searches
- Use `compass path "<source>" "<target>"` for dependency paths
- Use `compass explain "<concept>"` for one concept and its neighbors
- Use `compass affected "<symbol>"` for change-review scope
- Read `compass-out/GRAPH_REPORT.md` for broad architecture
- Navigate `compass-out/wiki/index.md` when the wiki exists
- Run `compass update .` after code changes
- Verify important graph conclusions in the cited source
- Treat missing paths and inferred edges as uncertain evidence, not proof
- Keep explicit `--graph`, `--at`, provider, and output selections unchanged
- Report failed refreshes; an older graph file does not make a failed update current
