import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { graphStaticLoadingMarkup } from "../../../editors/vscode/src/webviews/graphLoadingMarkup";
import { ArchitectureIndex } from "../../../editors/vscode/src/views/architectureIndex";
import { CallflowViewModelSchema } from "../../../packages/compass-viewer/src/contracts/callflow";

export default async function generate(): Promise<void> {
  const root = path.resolve("../..");
  execFileSync("npm", ["run", "build:viewer"], { cwd: root, stdio: "inherit" });
  execFileSync("npm", ["run", "build:vscode"], { cwd: root, stdio: "inherit" });
  const output = path.resolve("fixtures/out");
  await mkdir(output, { recursive: true });
  await cp(path.join(root, "packages/compass-viewer/dist/viewer.css"), path.join(output, "viewer.css"));
  await cp(path.join(root, "packages/compass-viewer/dist/graph.js"), path.join(output, "graph.js"));
  await cp(
    path.join(root, "crates/compass-cli/assets/semantic-diff-graph.css"),
    path.join(output, "semantic-diff-graph.css")
  );
  await cp(
    path.join(root, "crates/compass-cli/assets/semantic-diff-graph.js"),
    path.join(output, "semantic-diff-graph.js")
  );
  for (const name of ["architecture", "callGraph", "history", "initialize", "query"]) {
    await cp(
      path.join(root, `editors/vscode/dist/webviews/${name}.js`),
      path.join(output, `${name}.js`)
    );
  }
  await cp(
    path.join(root, "editors/vscode/dist/webviews/graph.js"),
    path.join(output, "vscodeGraph.js")
  );
  await writeFile(path.join(output, "loading.html"), graphLoadingHarness());
  const graph = {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes: 4, edges: 3, communities: 2, aggregated: false },
    nodes: [
      {
        id: "run", label: "run", kind: "function", community: 0, degree: 2,
        language: "rust", signature: "pub fn run(value: usize)", size: 30,
        source: { file: "src/lib.rs", startLine: 1, endLine: 3 }
      },
      {
        id: "helper", label: "helper", kind: "function", community: 0, degree: 2,
        language: "rust", signature: "fn helper()", size: 24,
        source: { file: "src/lib.rs", startLine: 5, endLine: 7 },
        codeEvidence: [{
          layer: "structural_graph",
          origin: "ast",
          extractor: "rust.functions",
          confidence: "exact",
          anchor: {
            file: "src/lib.rs", startByte: 40, endByte: 64,
            startLine: 5, startColumn: 0, endLine: 7, endColumn: 1
          },
          rule: null,
          wiringSite: null,
          resolution: "exact",
          candidates: []
        }]
      },
      {
        id: "file-only", label: "README", kind: "document", community: 1, degree: 1,
        source: { file: "README.md" }
      },
      { id: "store", label: "Store", kind: "type", community: 1, degree: 1 }
    ],
    edges: [
      { id: "e1", source: "run", target: "helper", relation: "calls", confidence: "extracted" },
      {
        id: "e2", source: "helper", target: "store", relation: "uses",
        confidence: "inferred",
        codeEvidence: [{
          layer: "structural_graph",
          origin: "heuristic",
          extractor: "rust.dynamic-dispatch",
          confidence: "ambiguous",
          anchor: null,
          rule: "trait-object-call",
          wiringSite: {
            file: "src/lib.rs", startByte: 52, endByte: 60,
            startLine: 6, startColumn: 2, endLine: 6, endColumn: 10
          },
          resolution: "ambiguous",
          candidates: [{
            nodeId: "store",
            reason: "compatible receiver type",
            confidence: "ambiguous"
          }]
        }]
      },
      { id: "e3", source: "run", target: "file-only", relation: "documents", confidence: "ambiguous" }
    ],
    communities: [
      { id: 0, label: "Core", color: "#4E79A7", hidden: false },
      { id: 1, label: "Data", color: "#F28E2B", hidden: false }
    ],
    hyperedges: []
  };
  const viewerJs = await readFile(path.join(output, "graph.js"), "utf8");
  const viewerCss = await readFile(path.join(output, "viewer.css"), "utf8");
  await writeFile(path.join(output, "graph.html"), `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass graph fixture</title><style>${viewerCss}</style></head><body><div id="compass-viewer-root"></div><script id="compass-viewer-model" type="application/json">${JSON.stringify(graph)}</script><script>${viewerJs}</script></body></html>`);
  const workbench = {
    schema: "compass.viewer.workbench/1",
    title: "Fixture workbench",
    graphIdentity: "fixture-workbench",
    defaultView: "code",
    views: [{
      id: "code",
      kind: "code",
      title: "Code graph",
      description: "Repository structure and relationships",
      coverage: {
        status: "complete",
        truncated: false,
        nodes: graph.nodes.length,
        edges: graph.edges.length,
        limitations: []
      },
      model: graph,
      communityDetails: {}
    }]
  };
  await writeFile(
    path.join(output, "workbench.html"),
    `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass workbench fixture</title><style>${viewerCss}</style></head><body><div id="compass-viewer-root"></div><script id="compass-viewer-model" type="application/json">${JSON.stringify(workbench)}</script><script>${viewerJs}</script></body></html>`
  );
  // Mirrors the prepared Django overview shape (3,376 communities and roughly
  // three cross-community relationships per community) without carrying the
  // original 281 MB graph fixture in the repository.
  const largeNodeCount = 3_400;
  const largeEdgeCount = 10_500;
  const largeGraph = {
    schema: "compass.viewer.graph/1",
    title: "Large graph fixture",
    stats: {
      nodes: largeNodeCount,
      edges: largeEdgeCount,
      communities: largeNodeCount,
      aggregated: true
    },
    nodes: Array.from({ length: largeNodeCount }, (_, index) => ({
      id: `large-node-${index}`,
      label: `Large node ${index}`,
      kind: "community",
      community: index,
      degree: index % 41,
      memberCount: 1 + index % 97
    })),
    edges: Array.from({ length: largeEdgeCount }, (_, index) => ({
      id: `large-edge-${index}`,
      source: `large-node-${index % largeNodeCount}`,
      target: `large-node-${(index * 17 + 1) % largeNodeCount}`,
      relation: `${1 + index % 13} cross-community edges`,
      confidence: "aggregated"
    })),
    communities: Array.from({ length: largeNodeCount }, (_, index) => ({
      id: index,
      label: `Community ${index}`,
      color: ["#4E79A7", "#F28E2B", "#E15759", "#76B7B2"][index % 4],
      hidden: false
    })),
    hyperedges: []
  };
  await writeFile(
    path.join(output, "largeGraph.html"),
    `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Large Compass graph fixture</title><style>${viewerCss}</style></head><body><div id="compass-viewer-root"></div><script id="compass-viewer-model" type="application/json">${JSON.stringify(largeGraph)}</script><script>${viewerJs}</script></body></html>`
  );
  await writeFile(
    path.join(output, "semanticDiffGraph.html"),
    semanticDiffGraphHarness()
  );
  const communityOverview = {
    schema: "compass.viewer.graph/1",
    title: "Community fixture",
    stats: { nodes: 2, edges: 1, communities: 2, aggregated: true },
    nodes: [
      { id: "0", label: "Core", community: 0, memberCount: 2, degree: 1 },
      { id: "1", label: "Data", community: 1, memberCount: 1, degree: 1 }
    ],
    edges: [{
      id: "overview-edge",
      source: "0",
      target: "1",
      relation: "2 cross-community edges",
      confidence: "aggregated"
    }],
    communities: [
      { id: 0, label: "Core", color: "#4E79A7", hidden: false },
      { id: 1, label: "Data", color: "#F28E2B", hidden: false }
    ],
    hyperedges: []
  };
  const communityDetail = {
    ...graph,
    title: "Core detail",
    stats: { nodes: 2, edges: 1, communities: 1, aggregated: false },
    nodes: [graph.nodes[0]!, { ...graph.nodes[3]!, community: 0 }],
    edges: [{
      id: "detail-edge",
      source: "run",
      target: "store",
      relation: "uses",
      confidence: "extracted"
    }],
    communities: graph.communities.slice(0, 1)
  };
  await writeFile(
    path.join(output, "community.html"),
    communityHarness(communityOverview, communityDetail)
  );
  await writeFile(
    path.join(output, "exportCommunity.html"),
    `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass export community fixture</title><style>${viewerCss}</style></head><body><div id="compass-viewer-root"></div><script id="compass-viewer-model" type="application/json">${JSON.stringify(communityOverview)}</script><script type="application/json" data-compass-community="0">${JSON.stringify(communityDetail)}</script><script>${viewerJs}</script></body></html>`
  );
  const architectureNodes = Array.from({ length: 31 }, (_, index) => ({
    id: index === 0 ? "authenticate" : index === 1 ? "database" : `symbol-${index}`,
    label: index === 0 ? "authenticate" : index === 1 ? "database" : `symbol${index}`,
    kind: index % 5 === 0 ? "class" : "function",
    sourceFile: `src/api/module-${index}.ts`,
    scope: "production"
  }));
  const architectureEdges = Array.from({ length: 53 }, (_, index) => ({
    source: "authenticate",
    target: index % 4 === 0 ? "database" : `symbol-${2 + (index % 29)}`,
    relation: index % 3 === 0 ? "calls" : "uses",
    confidence: index % 5 === 0 ? "inferred" : "extracted"
  }));
  const architectureSectionNodeId = (index: number) =>
    index === 0 ? "authenticate" : index === 1 ? "database-adapter" : `section-node-${index}`;
  const architectureSections = Array.from({ length: 26 }, (_, index) => {
    const nodes = index === 0
      ? architectureNodes
      : index === 1
        ? [{
          id: "database-adapter",
          label: "database adapter",
          kind: "class",
          sourceFile: "src/storage/database.ts",
          scope: "production"
        }]
        : [{
          id: architectureSectionNodeId(index),
          label: `section ${index} entry`,
          kind: "function",
          sourceFile: `src/section-${index}/entry.ts`,
          scope: "production"
        }];
    const edges = index === 0 ? architectureEdges : [];
    return {
      id: `section-${index}`,
      name: index === 0 ? "API" : index === 1 ? "Storage" : `Section ${index}`,
      communities: [`${index}`],
      nodeCount: nodes.length,
      internalCallCount: edges.length,
      nodes,
      edges
    };
  });
  const architectureOverviewLinks = Array.from({ length: 25 }, (_, index) => ({
    sourceSection: `section-${index}`,
    targetSection: `section-${index + 1}`,
    calls: index + 1
  }));
  const architectureCrossSectionCalls = architectureOverviewLinks.flatMap((link, index) =>
    Array.from({ length: link.calls }, () => ({
      source: architectureSectionNodeId(index),
      target: architectureSectionNodeId(index + 1),
      sourceSection: link.sourceSection,
      targetSection: link.targetSection,
      relation: "calls",
      confidence: "extracted"
    }))
  );
  const architecture = {
    schema: "compass.viewer.callflow/1",
    title: "Fixture — Architecture Flow",
    sections: [
      {
        id: "overview",
        name: "Overview",
        communities: [],
        nodeCount: 0,
        internalCallCount: 0,
        nodes: [],
        edges: []
      },
      ...architectureSections
    ],
    overviewLinks: architectureOverviewLinks,
    crossSectionCalls: architectureCrossSectionCalls,
    coverage: { internal: 53, crossSection: 325, unassigned: 0 },
    reportHighlights: [],
    statistics: {
      nodes: 56,
      edges: 378,
      communities: 26,
      hyperedges: 0,
      extracted: 367,
      inferred: 11,
      ambiguous: 0
    },
    provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
  };
  const calls = {
    schema: "compass.program.call_graph/1",
    rootSymbol: "run",
    direction: "both",
    depth: 1,
    nodes: [
      { id: "run", symbol: "run", name: "run", file: "src/lib.rs", anchor: { source_file: "src/lib.rs", start_byte: 0, end_byte: 10 }, graphNodeId: "run", unresolved: false },
      { id: "helper", symbol: "helper", name: "helper", file: "src/lib.rs", anchor: { source_file: "src/lib.rs", start_byte: 11, end_byte: 20 }, graphNodeId: "helper", unresolved: false }
    ],
    edges: [{
      id: "run-helper", source: "run", target: "helper", callee: "helper",
      resolution: "resolved",
      callSites: [{ anchor: { source_file: "src/lib.rs", start_byte: 4, end_byte: 10 }, evidence: ["fixture"] }]
    }],
    truncated: true,
    continuations: Array.from({ length: 21 }, (_, index) => ({
      symbol: `continuation-${index}`,
      direction: index % 2 === 0 ? "callers" : "callees",
      nextDepth: 2
    })),
    coverage: { resolved: 1, inferred: 0, ambiguous: 0, unresolved: 0, warning: "Unresolved calls never prove absence." }
  };
  const commitA = "a".repeat(40);
  const commitB = "b".repeat(40);
  const commitC = "c".repeat(40);
  const timeline = {
    schema: "compass.history.timeline/1",
    repositoryId: "fixture",
    selectedHead: commitA,
    historyEnabled: true,
    entries: [
      {
        commit: commitA, parents: [], authorName: "Compass", authorEmail: "test@example.invalid",
        authoredAtSeconds: 1, subject: "Revision A graph", graphState: "graph_available",
        presentationAvailable: true, realization: "ra", fingerprint: "fa", job: null
      },
      {
        commit: commitB, parents: [commitA], authorName: "Compass", authorEmail: "test@example.invalid",
        authoredAtSeconds: 2, subject: "Revision B graph", graphState: "graph_available",
        presentationAvailable: true, realization: "rb", fingerprint: "fb", job: null
      },
      {
        commit: commitC, parents: [commitB], authorName: "Compass", authorEmail: "test@example.invalid",
        authoredAtSeconds: 3, subject: "Revision C needs build", graphState: "not_materialized",
        presentationAvailable: false, realization: null, fingerprint: null, job: null
      }
    ]
  };
  const historyOverviewA = { ...communityOverview, title: "Revision A graph" };
  const historyOverviewB = {
    ...communityOverview,
    title: "Revision B graph",
    nodes: communityOverview.nodes.map((node) => ({
      ...node,
      label: `${node.label} B`
    })),
    communities: communityOverview.communities.map((community) => ({
      ...community,
      label: `${community.label} B`
    }))
  };
  const historyOverviewC = {
    ...historyOverviewB,
    title: "Revision C graph"
  };
  const historyCommunityA = {
    ...communityDetail,
    title: "Core detail at revision A",
    stats: { nodes: 3, edges: 1, communities: 1, aggregated: false },
    nodes: [
      {
        id: "run", label: "run", kind: "function", community: 0, degree: 1,
        language: "rust", signature: "pub fn run(old_value: usize)",
        source: { file: "src/lib.rs", startLine: 1, endLine: 3 }
      },
      {
        id: "shared", label: "shared", kind: "function", community: 0, degree: 1,
        language: "rust", signature: "fn shared()",
        source: { file: "src/shared.rs", startLine: 4, endLine: 6 }
      },
      {
        id: "removed", label: "removed_symbol", kind: "function", community: 0,
        source: { file: "src/removed.rs", startLine: 1 }
      }
    ],
    edges: [{
      id: "run-shared", source: "run", target: "shared", relation: "calls",
      confidence: "inferred"
    }],
    communities: communityOverview.communities.slice(0, 1)
  };
  const historyCommunityB = {
    ...historyCommunityA,
    title: "Core detail at revision B",
    nodes: [
      {
        id: "run", label: "run", kind: "function", community: 0, degree: 1,
        language: "rust", signature: "pub fn run(new_value: usize)",
        source: { file: "src/lib.rs", startLine: 8, endLine: 11 }
      },
      historyCommunityA.nodes[1],
      {
        id: "added", label: "added_symbol", kind: "function", community: 0,
        source: { file: "src/added.rs", startLine: 1 }
      }
    ],
    edges: [{
      id: "run-shared", source: "run", target: "shared", relation: "uses",
      confidence: "extracted"
    }]
  };
  await writeFile(path.join(output, "architecture.html"), architectureHarness(architecture));
  await writeFile(path.join(output, "calls.html"), callGraphHarness(calls));
  await writeFile(
    path.join(output, "history.html"),
    historyHarness(
      timeline,
      {
        [commitA]: historyOverviewA,
        [commitB]: historyOverviewB,
        [commitC]: historyOverviewC
      },
      {
        [commitA]: historyCommunityA,
        [commitB]: historyCommunityB,
        [commitC]: historyCommunityB
      }
    )
  );
  await writeFile(path.join(output, "query.html"), queryHarness());
  await writeFile(path.join(output, "initialize.html"), initializationHarness());
}

function semanticDiffGraphHarness(): string {
  const overflowEdges = Array.from({ length: 44 }, (_, index) => ({
    source: "changed-core",
    target: `overflow-context-${String(index).padStart(2, "0")}`,
    relation: "calls",
    key: `overflow-${index}`,
    source_file: "src/core.ts",
    changed_fields: []
  }));
  const report = {
    schema: "compass.semantic_diff.report/1",
    comparison: {
      old_commit: "a".repeat(40),
      new_commit: "b".repeat(40),
      fingerprint: "c".repeat(64)
    },
    findings: [{
      id: "sd1-fixture",
      subject: "changed-core",
      headline: "Changed core behavior",
      evidence: [{ record_key: "changed-core" }]
    }],
    source_changes: [{
      old_path: "src/core.ts",
      new_path: "src/core.ts",
      patch: "@@ -1 +1 @@\n-old\n+new"
    }],
    graph_delta: {
      changed_nodes: [
        {
          id: "changed-core",
          label: "changed-core",
          kind: "function",
          source_file: "src/core.ts",
          changed_fields: ["implementation"]
        },
        {
          id: "hostile",
          label: "</script><img src=x onerror=alert(1)>",
          kind: "function",
          source_file: "src/hostile.ts",
          changed_fields: ["body"]
        }
      ],
      added_nodes: [
        {
          id: "added-leaf",
          label: "added-leaf",
          kind: "function",
          source_file: "src/leaf.ts",
          changed_fields: []
        },
        {
          id: "unrelated",
          label: "unrelated",
          kind: "type",
          source_file: "src/unrelated.ts",
          changed_fields: []
        }
      ],
      removed_nodes: [
        {
          id: "removed-caller",
          label: "removed-caller",
          kind: "function",
          source_file: "src/caller.ts",
          changed_fields: []
        },
        ...Array.from({ length: 44 }, (_, index) => ({
          id: `removed-overflow-${String(index).padStart(2, "0")}`,
          label: `removed-overflow-${String(index).padStart(2, "0")}`,
          kind: "function",
          source_file: "src/removed.ts",
          changed_fields: []
        }))
      ],
      changed_edges: [{
        source: "removed-caller",
        target: "changed-core",
        relation: "calls",
        key: "removed-core",
        source_file: "src/core.ts",
        changed_fields: ["confidence"]
      }],
      added_edges: [
        {
          source: "changed-core",
          target: "added-leaf",
          relation: "calls",
          key: "core-leaf",
          source_file: "src/core.ts",
          changed_fields: []
        },
        {
          source: "changed-core",
          target: "context-api",
          relation: "uses",
          key: "core-context",
          source_file: "src/core.ts",
          changed_fields: []
        },
        ...overflowEdges,
        {
          source: "changed-core",
          target: "zz-outside-sample",
          relation: "calls",
          key: "outside",
          source_file: "src/core.ts",
          changed_fields: []
        }
      ],
      removed_edges: [],
      collapsed_attribute_changes: {}
    }
  };
  const rows = [
    ...report.graph_delta.changed_nodes,
    ...report.graph_delta.added_nodes,
    ...report.graph_delta.removed_nodes
  ].map((node) =>
    `<li class="delta-row" data-graph-node-id="${node.id}">${node.id}</li>`
  ).join("");
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Semantic diff graph fixture</title>
  <style>
    :root{color-scheme:dark;--canvas:#0e1116;--surface:#141820;--surface-raised:#191e27;--surface-inset:#0b0e13;--border:#2b313b;--border-strong:#3a424e;--text:#e7eaf0;--text-soft:#c5cad3;--muted:#8d96a5;--accent:#8ab4f8;--red:#ff7b86;--amber:#d9a441;--green:#65bd84}
    *{box-sizing:border-box}
    body{margin:0;padding:20px;background:var(--canvas);color:var(--text);font:14px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}
    button{font:inherit;color:inherit}
    .graph-note{color:var(--muted);font-size:11px}
    .delta-row{padding:5px}
  </style>
  <link rel="stylesheet" href="/semantic-diff-graph.css">
</head>
<body>
  <article id="sd1-fixture">Changed core behavior</article>
  <details id="source-change-0"><summary>src/core.ts</summary><pre>source patch</pre></details>
  <div class="graph-explorer">
    <div id="graph-canvas" class="graph-canvas" aria-label="Changed code graph"></div>
    <aside id="graph-inspector" class="graph-inspector" aria-labelledby="graph-inspector-heading">
      <h3 id="graph-inspector-heading" class="sr-only">Node inspector</h3>
      <p class="graph-inspector-empty">Select a node to inspect its change.</p>
    </aside>
  </div>
  <p id="graph-live" class="sr-only" aria-live="polite"></p>
  <p id="graph-note" class="graph-note">The visualization focuses on the changed subgraph.</p>
  <ul id="exhaustive-list">${rows}</ul>
  <script id="semantic-diff-data" type="application/json">${JSON.stringify(report).replaceAll("<", "\\u003c")}</script>
  <script src="/semantic-diff-graph.js"></script>
  <script>
    const report = JSON.parse(document.getElementById("semantic-diff-data").textContent);
    globalThis.graphFixture = globalThis.CompassSemanticDiffGraph.mount({
      report,
      host: document.getElementById("graph-canvas"),
      inspector: document.getElementById("graph-inspector"),
      liveRegion: document.getElementById("graph-live"),
      note: document.getElementById("graph-note")
    });
  </script>
</body>
</html>`;
}

function graphLoadingHarness(): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass graph loading fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root">${graphStaticLoadingMarkup()}</div><script>
window.hostMessages=[];
window.webviewState=undefined;
window.acquireVsCodeApi=()=>({
  getState(){return window.webviewState},
  setState(state){window.webviewState=state},
  postMessage(message){
    window.hostMessages.push(message);
    const params=new URLSearchParams(window.location.search);
    if((params.has("large") || params.has("snapshot")) && message.type==="ready") {
      setTimeout(()=>window.postMessage({
        type:"graphLoadStatus",
        mode:"large",
        graphBytes:44275915,
        phase:params.has("snapshot") ? "snapshotting" : "exporting"
      },"*"),0);
    }
    if(params.has("error") && (message.type==="ready" || message.type==="retry")) {
      setTimeout(()=>window.postMessage({type:"error",message:"The graph export could not be read."},"*"),0);
    }
  }
})</script><script src="/vscodeGraph.js"></script></body></html>`;
}

function callGraphHarness(graph: unknown): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass call graph fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.callGraphHostMessages=[];
window.acquireVsCodeApi=()=>({postMessage(message){
  window.callGraphHostMessages.push(message);
  if(message.type==="openSource") {
    window.openedCallGraphSource=message.source;
    return;
  }
  if(message.type==="showOutput") {
    window.showedCallGraphOutput=true;
    return;
  }
  if(message.type!=="ready" && message.type!=="retry") return;
  if(new URLSearchParams(window.location.search).has("error")) {
    setTimeout(()=>window.postMessage({type:"error",message:"No function could be resolved at this cursor position."},"*"),20);
    return;
  }
  setTimeout(()=>window.postMessage({
    type:"hydrateCallGraph",
    repositoryId:"fixture",
    graph:${JSON.stringify(graph)}
  },"*"),1000);
}})</script><script src="/callGraph.js"></script></body></html>`;
}

function architectureHarness(model: unknown): string {
  const parsedModel = CallflowViewModelSchema.parse(model);
  const index = new ArchitectureIndex(parsedModel);
  const overview = index.overview("production", "all");
  const sectionPages: Record<string, unknown> = {};
  for (const section of overview.sections) {
    for (const kind of ["symbols", "calls"] as const) {
      for (const query of ["", "database"]) {
        for (const page of [1, 2]) {
          const key = [section.id, kind, page, query].join("|");
          sectionPages[key] = index.sectionPage({
            sectionId: section.id,
            kind,
            page,
            pageSize: 100,
            query,
            scope: "production",
            evidence: "all"
          });
        }
      }
    }
  }
  const routePages = Object.fromEntries(
    overview.routes.map((route) => [
      [route.id, 1, ""].join("|"),
      index.routePage({
        routeId: route.id,
        page: 1,
        pageSize: 100,
        query: "",
        scope: "production",
        evidence: "all"
      })
    ])
  );
  const searchPages = Object.fromEntries(
    ["", "database"].map((query) => [
      [1, query].join("|"),
      index.search({
        query,
        page: 1,
        pageSize: 100,
        scope: "production",
        evidence: "all"
      })
    ])
  );
  const fixture = { overview, sectionPages, routePages, searchPages };
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass architecture fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.architectureHostMessages=[];
const architectureFixture=${JSON.stringify(fixture)};
const architectureIdentity=(message)=>({
  requestId:message.requestId||"fixture-bootstrap",
  repositoryId:"fixture",
  generation:1
});
const sendArchitecture=(message,delay=0)=>
  setTimeout(()=>window.postMessage(message,"*"),delay);
window.acquireVsCodeApi=()=>({postMessage(message){
  window.architectureHostMessages.push(message);
  if(message.type==="showOutput") {
    window.showedArchitectureOutput=true;
    return;
  }
  if(message.type==="openSource") {
    window.openedArchitectureSource=message.file;
    return;
  }
  const params=new URLSearchParams(window.location.search);
  if((message.type==="ready" || message.type==="retry") && params.has("error")) {
    setTimeout(()=>window.postMessage({type:"error",message:"Architecture export failed"},"*"),20);
    return;
  }
  if(message.type==="ready" || message.type==="retry") {
    const delayed=params.has("delay");
    if(delayed) sendArchitecture({
      type:"architectureLoading",
      phase:"indexing",
      message:"Preparing symbol index"
    },20);
    sendArchitecture({
      type:"architectureOverview",
      ...architectureIdentity(message),
      model:architectureFixture.overview
    },delayed ? 800 : 20);
    return;
  }
  if(message.type==="requestSection") {
    const key=[message.sectionId,message.kind,message.page,message.query||""].join("|");
    sendArchitecture({
      type:"architectureSectionPage",
      ...architectureIdentity(message),
      model:architectureFixture.sectionPages[key]
    });
    return;
  }
  if(message.type==="requestRoute") {
    const key=[message.routeId,message.page,message.query||""].join("|");
    sendArchitecture({
      type:"architectureRoutePage",
      ...architectureIdentity(message),
      model:architectureFixture.routePages[key]
    });
    return;
  }
  if(message.type==="searchArchitecture") {
    const key=[message.page,message.query].join("|");
    sendArchitecture({
      type:"architectureSearchResults",
      ...architectureIdentity(message),
      model:architectureFixture.searchPages[key]
    });
    return;
  }
  if(message.type==="setArchitectureFilters") {
    sendArchitecture({
      type:"architectureOverview",
      ...architectureIdentity(message),
      model:{...architectureFixture.overview,scope:message.scope,evidence:message.evidence}
    });
  }
}})</script><script src="/architecture.js"></script></body></html>`;
}

function queryHarness(): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass query fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.queryHostMessages=[];
window.queryTimers=new Map();
window.queryCompletionTimers=new Map();
const queryLimits={
  maxDepth:8,maxNodes:500,maxEdges:1000,maxPaths:100,maxCandidates:20,
  maxSourceBytes:1048576,maxResponseBytes:8388608
};
const queryNode={
  id:"pipeline",kind:"function",roles:[],name:"Pipeline",
  qualifiedName:"caching::util::Pipeline",language:"scala",framework:null,
  source:{
    file:"caching/util/src/Pipeline.scala",startByte:0,endByte:8,
    startLine:154,startColumn:0,endLine:154,endColumn:8
  },
  details:null,evidence:[]
};
const typedQueryResult=(diagnostic=false)=>({
  schema:"compass.query/1",operation:"search",
  results:diagnostic?[]:[{nodeId:"pipeline",score:1,matchedFields:["qualifiedName"]}],
  nodes:diagnostic?[]:[queryNode],edges:[],files:[],paths:[],
  diagnostics:diagnostic?[{
    code:"no_match",
    message:"No exact node matched. Try a qualified name such as caching::util::Pipeline.",
    nodeId:null,path:null
  }]:[],
  limits:queryLimits,truncated:false
});
window.acquireVsCodeApi=()=>({postMessage(message){
  window.queryHostMessages.push(message);
  if(message.type==="complete") {
    const params=new URLSearchParams(window.location.search);
    const requestId=message.request.id;
    const timer=setTimeout(()=>{
      window.queryCompletionTimers.delete(requestId);
      if(params.has("completionError")) {
        window.postMessage({
          type:"completionError",requestId,message:"Fixture graph search failed"
        },"*");
        return;
      }
      const term=message.request.term.toLocaleLowerCase();
      const matches=[queryNode.name,queryNode.qualifiedName,queryNode.id]
        .some(value=>value.toLocaleLowerCase().includes(term));
      window.postMessage({
        type:"completions",requestId,
        items:matches?[{
          nodeId:queryNode.id,label:queryNode.qualifiedName,
          insertText:queryNode.qualifiedName,
          detail:"function · caching/util/src/Pipeline.scala:154"
        }]:[]
      },"*");
    },params.has("completionDelay")?800:20);
    window.queryCompletionTimers.set(requestId,timer);
    return;
  }
  if(message.type==="cancelCompletion") {
    clearTimeout(window.queryCompletionTimers.get(message.requestId));
    window.queryCompletionTimers.delete(message.requestId);
    window.postMessage({type:"completionCancelled",requestId:message.requestId},"*");
    return;
  }
  if(message.type==="openSource") {
    window.openedQuerySource=message.source;
    return;
  }
  if(message.type==="openGraph") {
    window.openedQueryGraph=true;
    return;
  }
  if(message.type==="ready") {
    setTimeout(()=>window.postMessage({type:"state",revision:"fixture-revision"},"*"),0);
    return;
  }
  if(message.type==="cancel") {
    clearTimeout(window.queryTimers.get(message.runId));
    window.queryTimers.delete(message.runId);
    window.postMessage({type:"cancelled",runId:message.runId},"*");
    return;
  }
  if(message.type!=="execute") return;
  const params=new URLSearchParams(window.location.search);
  const delay=params.has("delay") ? 1200 : 20;
  const runId=message.request.id;
  const timer=setTimeout(()=>{
    window.queryTimers.delete(runId);
    if(params.has("error")) {
      window.postMessage({
        type:"error",runId,message:"CompassQL could not parse this query"
      },"*");
    } else if(message.request.command==="cql") {
      window.postMessage({
        type:"result",runId,
        output:{kind:"rows",value:{rows:[{symbol:"run",calls:3},{symbol:"save",calls:2}]}},
        durationMs:18
      },"*");
    } else if(message.request.command==="explain") {
      window.postMessage({
        type:"result",runId,
        output:{kind:"explanation",text:"Node: Pipeline\\n  ID:        pipeline\\n  Source:    caching/util/src/Pipeline.scala L154\\n  Type:      function\\n  Community: Caching\\n  Degree:    2\\n\\nConnections (2):\\n  --> save [calls] [exact]\\n  <-- run [calls] [inferred]"},
        durationMs:12
      },"*");
    } else {
      window.postMessage({
        type:"result",runId,
        output:{kind:"code-query",value:typedQueryResult(params.has("diagnostic"))},
        durationMs:24
      },"*");
    }
  },delay);
  window.queryTimers.set(runId,timer);
}})</script><script src="/query.js"></script></body></html>`;
}

function initializationHarness(): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass initialization fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.initializationHostMessages=[];
window.initializationTimers=[];
window.acquireVsCodeApi=()=>({postMessage(message){
  window.initializationHostMessages.push(message);
  if(message.type==="ready" || message.type==="reset") {
    setTimeout(()=>window.postMessage({
      type:"hydrate",
      repositoryName:"compass",
      repositoryRoot:"/workspace/compass",
      scopeFiles:new URLSearchParams(window.location.search).has("manyFiles")
        ? [
          "packages/api/src/main.ts",
          ...Array.from({length:180},(_,index)=>
            "src/module-"+String(index).padStart(3,"0")+".ts")
        ]
        : [
          "packages/api/src/main.ts",
          "src/commands/init.ts",
          "src/core/index.ts",
          "src/views/graph.ts"
        ],
      scopeFilesTruncated:false,
      configurationExists:new URLSearchParams(window.location.search).has("existing")
    },"*"),0);
    return;
  }
  if(message.type==="cancel") {
    for(const timer of window.initializationTimers) clearTimeout(timer);
    window.postMessage({type:"cancelled"},"*");
    return;
  }
  if(message.type!=="start") return;
  const progress=(delay,current,total,file)=>window.initializationTimers.push(setTimeout(
    ()=>window.postMessage({
      type:"progress",
      event:{phase:"indexing",current,total,message:file}
    },"*"),
    delay
  ));
  progress(40,1,3,"src/commands/init.ts");
  progress(440,2,3,"src/core/index.ts");
  progress(840,3,3,"src/views/graph.ts");
  window.completeInitialization=()=>window.postMessage({
    type:"succeeded",
    message:"compass is indexed and ready for graph exploration."
  },"*");
  if(!new URLSearchParams(window.location.search).has("manualSuccess")) {
    window.initializationTimers.push(setTimeout(window.completeInitialization,1240));
  }
}})</script><script src="/initialize.js"></script></body></html>`;
}

function communityHarness(overview: unknown, detail: unknown): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass community fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.communityRequestCount=0;
window.webviewState=undefined;
window.acquireVsCodeApi=()=>({getState(){return window.webviewState},setState(state){window.webviewState=state},postMessage(message){
  if(message.type==="ready") {
    setTimeout(()=>window.postMessage({type:"hydrateGraph",requestId:"overview",repositoryId:"fixture",model:${JSON.stringify(overview)}},"*"),0);
  } else if(message.type==="openCommunity") {
    window.communityRequestCount+=1;
    window.openedCommunity=message.communityId;
    if(message.communityId===1 && window.communityRequestCount===1) {
      setTimeout(()=>window.postMessage({type:"communityError",requestId:message.requestId,communityId:message.communityId,message:"Community detail failed"},"*"),800);
    } else {
      setTimeout(()=>window.postMessage({type:"communityGraph",requestId:message.requestId,repositoryId:"fixture",communityId:message.communityId,model:${JSON.stringify(detail)}},"*"),500);
    }
  } else if(message.type==="openSource") {
    window.openedSource=message.source;
  }
}})</script><script src="/vscodeGraph.js"></script></body></html>`;
}

function historyHarness(
  timeline: { entries: Array<{ commit: string }> },
  graphs: Record<string, unknown>,
  details: Record<string, unknown>
): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass history fixture</title><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'self'; style-src-attr 'unsafe-inline'; script-src 'self' 'unsafe-inline';"><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.fixtureTimeline=${JSON.stringify(timeline)};
window.historyGraphs=${JSON.stringify(graphs)};
window.historyCommunityGraphs=${JSON.stringify(details)};
window.historyHostMessages=[];
window.pendingHistoryGraph=null;
window.historyBootstrapAttempts=0;
window.historyGeneration=0;
window.emitHistoryMessage=(message)=>window.postMessage(message,"*");
window.releaseHistoryGraph=()=>{
  if(!window.pendingHistoryGraph) {
    throw new Error("No history graph response is pending");
  }
  const response=window.pendingHistoryGraph;
  window.pendingHistoryGraph=null;
  window.postMessage(response,"*");
};
window.acquireVsCodeApi=()=>({postMessage(message){
  window.historyHostMessages.push(message);
  if(message.type==="ready" || message.type==="retryTimeline") {
    window.historyBootstrapAttempts+=1;
    const scenario=new URLSearchParams(window.location.search).get("bootstrap");
    if(scenario==="error" && window.historyBootstrapAttempts===1) {
      setTimeout(()=>window.postMessage({type:"bootstrapError",message:"Fixture history unavailable"},"*"),40);
    } else {
      window.historyGeneration+=1;
      const parameters=new URLSearchParams(window.location.search);
      const paginated=parameters.get("pagination")==="true";
      const baseTimeline=parameters.get("historyEnabled")==="false"
        ? {...window.fixtureTimeline,historyEnabled:false}
        : window.fixtureTimeline;
      const initialTimeline=paginated
        ? {...baseTimeline,totalEntries:null,hasMore:true,nextCursor:"fixture-cursor",entries:baseTimeline.entries.slice(0,2)}
        : baseTimeline;
      setTimeout(()=>window.postMessage({type:"timeline",repositoryId:"fixture",generation:window.historyGeneration,timeline:initialTimeline},"*"),40);
    }
  } else if(message.type==="loadMoreTimeline") {
    const pageGeneration=window.historyGeneration;
    setTimeout(()=>window.postMessage({
      type:"timelinePage",
      repositoryId:"fixture",
      generation:pageGeneration,
      timeline:{...window.fixtureTimeline,totalEntries:window.fixtureTimeline.entries.length,hasMore:false,nextCursor:null,entries:window.fixtureTimeline.entries.slice(2)}
    },"*"),40);
  } else if(message.type==="enableHistory") {
    setTimeout(()=>window.postMessage({type:"enableRunning"},"*"),20);
    setTimeout(()=>{
      window.historyGeneration+=1;
      window.postMessage({type:"timeline",repositoryId:"fixture",generation:window.historyGeneration,timeline:{...window.fixtureTimeline,historyEnabled:true}},"*");
      window.postMessage({type:"enableSucceeded"},"*");
    },120);
  } else if(message.type==="loadRevision") {
    const loadScenario=new URLSearchParams(window.location.search).get("load");
    const delay=message.commit.startsWith("a") ? 180 : 120;
    if(loadScenario==="error") {
      setTimeout(()=>window.postMessage({
        type:"error",
        operation:"Load graph",
        commit:message.commit,
        message:"Fixture graph load failed"
      },"*"),delay);
    } else {
      const response={
        type:"graph",
        commit:message.commit,
        realization:"r-"+message.commit.slice(0,1),
        fingerprint:"f-"+message.commit.slice(0,1),
        graph:window.historyGraphs[message.commit]
      };
      if(loadScenario==="manual") {
        window.pendingHistoryGraph=response;
      } else {
        setTimeout(()=>window.postMessage(response,"*"),delay);
      }
    }
  } else if(message.type==="changeCounts") {
    setTimeout(()=>window.postMessage({
      type:"changeCounts",
      commit:message.commit,
      counts:{
        schema:"compass.history.change_counts/1",
        commit:message.commit,
        parent:message.commit.startsWith("c") ? "b".repeat(40) : "a".repeat(40),
        counts:{
          nodes:{added:2,removed:0,changed:1},
          edges:{added:1,removed:0,changed:0},
          hyperedges:{added:0,removed:0,changed:0}
        }
      }
    },"*"),20);
  } else if(message.type==="openCommunity") {
    window.openedHistoricalCommunity=message.communityId;
    setTimeout(()=>window.postMessage({type:"communityGraph",requestId:message.requestId,commit:message.commit,communityId:message.communityId,graph:window.historyCommunityGraphs[message.commit]},"*"),0);
  } else if(message.type==="compare") {
    setTimeout(()=>window.postMessage({
      type:"comparison",
      commit:message.commit,
      parent:message.parent,
      realization:"r-"+message.commit.slice(0,1),
      fingerprint:"f-"+message.commit.slice(0,1),
      parentRealization:"r-"+message.parent.slice(0,1),
      parentFingerprint:"f-"+message.parent.slice(0,1),
      currentGraph:window.historyGraphs[message.commit],
      parentGraph:window.historyGraphs[message.parent],
      counts:{
        schema:"compass.history.change_counts/1",
        commit:message.commit,
        parent:message.parent,
        counts:{
          nodes:{added:0,removed:0,changed:1},
          edges:{added:0,removed:0,changed:0},
          hyperedges:{added:0,removed:0,changed:0}
        }
      },
      semanticDiff:{
        schema:"compass.semantic_diff.report/1",
        comparison:{old_commit:message.parent,new_commit:message.commit,fingerprint:"fixture"},
        source_changes:[{
          old_path:"Cargo.toml",
          new_path:"Cargo.toml",
          status:"modified",
          hunks:[{old_start:3,old_lines:3,new_start:3,new_lines:3}],
          patch:"@@ -3,3 +3,3 @@ [package]\\n name = \\"compass\\"\\n-version = \\"3.1.6\\"\\n+version = \\"3.1.7\\"\\n edition = \\"2021\\"\\n"
        }],
        findings:[{
          id:"sd1-fixture",
          finding_type:"behavior_change",
          subject:"run",
          origin:"direct",
          headline:"Fixture comparison",
          explanation:"The run signature changed.",
          compatibility:"indeterminate",
          confidence:"exact",
          review_priority:2,
          public_surface:true,
          routine:false,
          affected_consumers:[],
          witness_paths:[],
          verification:{state:"partial",exact_tests:[],recommended_tests:["run tests"],reason:"Fixture mapping is partial."},
          reviewer_action:"Review callers of run.",
          evidence:[{source_file:"src/lib.rs",record_key:"run",capability:"signature"}],
          completeness:{signature:"complete",test_mapping:"partial"}
        }],
        feature_groups:[],
        collapsed_groups:[],
        graph_delta:{
          added_nodes:[],
          removed_nodes:[],
          changed_nodes:[{
            id:"run",
            label:"run",
            kind:"function",
            source_file:"src/lib.rs",
            changed_fields:["signature"]
          }],
          added_edges:[],
          removed_edges:[],
          changed_edges:[],
          collapsed_attribute_changes:{}
        },
        entity_display_names:{run:"run"},
        completeness:{identity:"complete",source_delta:"complete",call_resolution:"partial",test_mapping:"partial"},
        limitations:["Fixture call mapping is partial."]
      }
    },"*"),0);
  } else if(message.type==="compareCommunity") {
    setTimeout(()=>window.postMessage({
      type:"communityComparison",
      requestId:message.requestId,
      commit:message.commit,
      parent:message.parent,
      communityId:message.communityId,
      currentGraph:message.hasCurrent ? window.historyCommunityGraphs[message.commit] : undefined,
      parentGraph:message.hasParent ? window.historyCommunityGraphs[message.parent] : undefined,
      nodeLimit:5000
    },"*"),0);
  } else if(message.type==="buildRevision") {
    const scenario=new URLSearchParams(window.location.search).get("build") || "cancel";
    if(scenario==="cancel") {
      setTimeout(()=>window.postMessage({type:"buildCancelled",commit:message.commit},"*"),180);
    } else {
      setTimeout(()=>window.postMessage({type:"buildRunning",commit:message.commit},"*"),100);
      if(scenario==="fail") {
        setTimeout(()=>window.postMessage({type:"buildFailed",commit:message.commit,message:"Fixture build failed"},"*"),220);
      } else {
        setTimeout(()=>{
          const entry=window.fixtureTimeline.entries.find(candidate=>candidate.commit===message.commit);
          Object.assign(entry,{graphState:"graph_available",presentationAvailable:true,realization:"rc",fingerprint:"fc"});
          window.historyGeneration+=1;
          window.postMessage({type:"timeline",repositoryId:"fixture",generation:window.historyGeneration,timeline:window.fixtureTimeline},"*");
          window.postMessage({type:"buildSucceeded",commit:message.commit},"*");
        },220);
      }
    }
  } else if(message.type==="openSource") {
    window.openedSource=message.source;
  }
}})</script><script src="/history.js"></script></body></html>`;
}
