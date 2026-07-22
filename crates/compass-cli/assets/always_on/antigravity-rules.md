---
trigger: always_on
description: Consult the graphify knowledge graph at compass-out/ for codebase and architecture questions.
---

## graphify

This project has a graphify knowledge graph at compass-out/.

Rules:
- For codebase or architecture questions, when `compass-out/graph.json` exists, first run `compass query "<question>"` (CLI) or `query_graph` (MCP). Use `compass path "<A>" "<B>"` / `shortest_path` for relationships and `compass explain "<concept>"` / `get_node` for focused concepts. These return a scoped subgraph, usually much smaller than `GRAPH_REPORT.md` or raw grep output.
- If compass-out/wiki/index.md exists, navigate it instead of reading raw files
- Read compass-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context
- After modifying code files in this session, run `compass update .` to keep the graph current (AST-only, no API cost)
