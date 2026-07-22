## graphify

This project has a knowledge graph at compass-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `compass query "<question>"` when compass-out/graph.json exists. Use `compass path "<A>" "<B>"` for relationships and `compass explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If compass-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read compass-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `compass update .` to keep the graph current (AST-only, no API cost).
