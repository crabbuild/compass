---
inclusion: always
---

graphify: A knowledge graph of this project lives in `compass-out/`. For codebase, architecture, or dependency questions, when `compass-out/graph.json` exists, first run `compass query "<question>"` (or `compass path "<A>" "<B>"` / `compass explain "<concept>"`). These return a scoped subgraph, usually much smaller than `GRAPH_REPORT.md` or raw grep output. Read `GRAPH_REPORT.md` only for broad architecture review or when those commands do not surface enough context.
