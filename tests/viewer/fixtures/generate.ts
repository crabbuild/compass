import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { execFileSync } from "node:child_process";

export default async function generate(): Promise<void> {
  const root = path.resolve("../..");
  execFileSync("npm", ["run", "build:viewer"], { cwd: root, stdio: "inherit" });
  execFileSync("npm", ["run", "build:vscode"], { cwd: root, stdio: "inherit" });
  const output = path.resolve("fixtures/out");
  await mkdir(output, { recursive: true });
  await cp(path.join(root, "packages/compass-viewer/dist/viewer.css"), path.join(output, "viewer.css"));
  await cp(path.join(root, "packages/compass-viewer/dist/graph.js"), path.join(output, "graph.js"));
  for (const name of ["architecture", "callGraph", "history", "query"]) {
    await cp(
      path.join(root, `editors/vscode/dist/webviews/${name}.js`),
      path.join(output, `${name}.js`)
    );
  }
  const graph = {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes: 3, edges: 2, communities: 2, aggregated: false },
    nodes: [
      { id: "run", label: "run", community: 0, degree: 2, source: { file: "src/lib.rs", startLine: 1 } },
      { id: "helper", label: "helper", community: 0, degree: 1, source: { file: "src/lib.rs", startLine: 5 } },
      { id: "store", label: "Store", community: 1, degree: 1 }
    ],
    edges: [
      { id: "e1", source: "run", target: "helper", relation: "calls", confidence: "extracted" },
      { id: "e2", source: "helper", target: "store", relation: "uses", confidence: "inferred" }
    ],
    communities: [
      { id: 0, label: "Core", color: "#4E79A7", hidden: false },
      { id: 1, label: "Data", color: "#59A14F", hidden: false }
    ],
    hyperedges: []
  };
  const viewerJs = await readFile(path.join(output, "graph.js"), "utf8");
  const viewerCss = await readFile(path.join(output, "viewer.css"), "utf8");
  await writeFile(path.join(output, "graph.html"), `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass graph fixture</title><style>${viewerCss}</style></head><body><div id="compass-viewer-root"></div><script id="compass-viewer-model" type="application/json">${JSON.stringify(graph)}</script><script>${viewerJs}</script></body></html>`);
  const architecture = {
    schema: "compass.viewer.callflow/1",
    title: "Fixture — Architecture Flow",
    sections: [
      { id: "overview", name: "Overview", communities: [], nodes: [], edges: [] },
      { id: "core", name: "Core", communities: ["0"], nodes: [
        { id: "run", label: "run", kind: "function", sourceFile: "src/lib.rs" }
      ], edges: [] }
    ],
    overviewLinks: [],
    reportHighlights: [],
    statistics: { nodes: 1, edges: 0, communities: 1, hyperedges: 0, extracted: 0, inferred: 0, ambiguous: 0 },
    provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
  };
  const calls = {
    schema: "compass.program.call_graph/1",
    rootSymbol: "run",
    direction: "both",
    depth: 1,
    nodes: [{ id: "run", symbol: "run", name: "run", file: "src/lib.rs", anchor: { source_file: "src/lib.rs", start_byte: 0, end_byte: 10 }, graphNodeId: "run", unresolved: false }],
    edges: [],
    truncated: false,
    continuations: [],
    coverage: { resolved: 0, inferred: 0, ambiguous: 0, unresolved: 0, warning: "Unresolved calls never prove absence." }
  };
  const timeline = {
    schema: "compass.history.timeline/1",
    repositoryId: "fixture",
    selectedHead: "a".repeat(40),
    historyEnabled: true,
    entries: [{
      commit: "a".repeat(40), parents: [], authorName: "Compass", authorEmail: "test@example.invalid",
      authoredAtSeconds: 1, subject: "Initial graph", graphState: "graph_available",
      presentationAvailable: true, realization: "r", fingerprint: "f", job: null
    }]
  };
  await writeFile(path.join(output, "architecture.html"), harness("architecture", { type: "hydrate", repositoryId: "fixture", model: architecture }));
  await writeFile(path.join(output, "calls.html"), harness("callGraph", { type: "hydrateCallGraph", repositoryId: "fixture", graph: calls }));
  await writeFile(path.join(output, "history.html"), harness("history", { type: "timeline", repositoryId: "fixture", timeline }));
  await writeFile(path.join(output, "query.html"), harness("query", { type: "state", running: false }));
}

function harness(script: string, hydration: unknown): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass ${script} fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>window.acquireVsCodeApi=()=>({postMessage(message){if(message.type==="ready")setTimeout(()=>window.postMessage(${JSON.stringify(hydration)},"*"),0)}})</script><script src="/${script}.js"></script></body></html>`;
}
