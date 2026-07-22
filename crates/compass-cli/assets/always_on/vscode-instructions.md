## graphify

For any question about this repo's architecture, structure, components, or how to add/modify/find
code, your first action should be `compass query "<question>"` when `compass-out/graph.json`
exists. Use `compass path "<A>" "<B>"` for relationship questions and `compass explain "<concept>"`
for focused-concept questions. These return a scoped subgraph, usually much smaller than the full
report or raw grep output.

Triggers: "how do I…", "where is…", "what does … do", "add/modify a <component>",
"explain the architecture", or anything that depends on how files or classes relate.

If `compass-out/wiki/index.md` exists, use it for broad navigation. Read `compass-out/GRAPH_REPORT.md`
only for broad architecture review or when query/path/explain do not surface enough context. Only read
source files when (a) modifying/debugging specific code, (b) the graph lacks the needed detail, or
(c) the graph is missing or stale.

Type `/compass` in Copilot Chat to build or update the graph.
