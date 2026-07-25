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
        source: { file: "src/lib.rs", startLine: 5, endLine: 7 }
      },
      {
        id: "file-only", label: "README", kind: "document", community: 1, degree: 1,
        source: { file: "README.md" }
      },
      { id: "store", label: "Store", kind: "type", community: 1, degree: 1 }
    ],
    edges: [
      { id: "e1", source: "run", target: "helper", relation: "calls", confidence: "extracted" },
      { id: "e2", source: "helper", target: "store", relation: "uses", confidence: "inferred" },
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
  const communityOverview = {
    schema: "compass.viewer.graph/1",
    title: "Community fixture",
    stats: { nodes: 2, edges: 1, communities: 2, aggregated: true },
    nodes: [
      { id: "0", label: "Core", community: 0, memberCount: 2, degree: 1 },
      { id: "1", label: "Data", community: 1, memberCount: 1, degree: 1 }
    ],
    edges: [{ id: "overview-edge", source: "0", target: "1", relation: "calls" }],
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
    nodes: graph.nodes.slice(0, 2),
    edges: graph.edges.slice(0, 1),
    communities: graph.communities.slice(0, 1)
  };
  await writeFile(
    path.join(output, "community.html"),
    communityHarness(communityOverview, communityDetail)
  );
  const architectureSections = Array.from({ length: 26 }, (_, index) => ({
    id: `section-${index}`,
    name: `Section ${index}`,
    communities: [`${index}`],
    nodes: index === 0
      ? [{ id: "run", label: "run", kind: "function", sourceFile: "src/lib.rs" }]
      : [],
    edges: []
  }));
  const architecture = {
    schema: "compass.viewer.callflow/1",
    title: "Fixture — Architecture Flow",
    sections: [
      { id: "overview", name: "Overview", communities: [], nodes: [], edges: [] },
      ...architectureSections
    ],
    overviewLinks: Array.from({ length: 25 }, (_, index) => ({
      sourceSection: `section-${index}`,
      targetSection: `section-${index + 1}`,
      calls: index + 1
    })),
    reportHighlights: [],
    statistics: { nodes: 1, edges: 0, communities: 1, hyperedges: 0, extracted: 0, inferred: 0, ambiguous: 0 },
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
  await writeFile(path.join(output, "architecture.html"), harness("architecture", { type: "hydrate", repositoryId: "fixture", model: architecture }));
  await writeFile(path.join(output, "calls.html"), harness("callGraph", { type: "hydrateCallGraph", repositoryId: "fixture", graph: calls }));
  await writeFile(
    path.join(output, "history.html"),
    historyHarness(
      timeline,
      {
        [commitA]: historyOverviewA,
        [commitB]: historyOverviewB,
        [commitC]: historyOverviewC
      },
      communityDetail
    )
  );
  await writeFile(path.join(output, "query.html"), harness("query", { type: "state", running: false }));
}

function graphLoadingHarness(): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass graph loading fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.hostMessages=[];
window.webviewState=undefined;
window.acquireVsCodeApi=()=>({
  getState(){return window.webviewState},
  setState(state){window.webviewState=state},
  postMessage(message){
    window.hostMessages.push(message);
    if(new URLSearchParams(window.location.search).has("error") && (message.type==="ready" || message.type==="retry")) {
      setTimeout(()=>window.postMessage({type:"error",message:"The graph export could not be read."},"*"),0);
    }
  }
})</script><script src="/vscodeGraph.js"></script></body></html>`;
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
      setTimeout(()=>window.postMessage({type:"communityError",requestId:message.requestId,communityId:message.communityId,message:"Community detail failed"},"*"),250);
    } else {
      setTimeout(()=>window.postMessage({type:"communityGraph",requestId:message.requestId,repositoryId:"fixture",communityId:message.communityId,model:${JSON.stringify(detail)}},"*"),0);
    }
  } else if(message.type==="openSource") {
    window.openedSource=message.source;
  }
}})</script><script src="/vscodeGraph.js"></script></body></html>`;
}

function historyHarness(
  timeline: { entries: Array<{ commit: string }> },
  graphs: Record<string, unknown>,
  detail: unknown
): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass history fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.fixtureTimeline=${JSON.stringify(timeline)};
window.historyGraphs=${JSON.stringify(graphs)};
window.historyHostMessages=[];
window.emitHistoryMessage=(message)=>window.postMessage(message,"*");
window.acquireVsCodeApi=()=>({postMessage(message){
  window.historyHostMessages.push(message);
  if(message.type==="ready") {
    setTimeout(()=>window.postMessage({type:"timeline",repositoryId:"fixture",timeline:window.fixtureTimeline},"*"),0);
  } else if(message.type==="loadRevision") {
    const delay=message.commit.startsWith("a") ? 180 : 0;
    setTimeout(()=>window.postMessage({
      type:"graph",
      commit:message.commit,
      realization:"r-"+message.commit.slice(0,1),
      fingerprint:"f-"+message.commit.slice(0,1),
      graph:window.historyGraphs[message.commit]
    },"*"),delay);
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
    setTimeout(()=>window.postMessage({type:"communityGraph",requestId:message.requestId,commit:message.commit,communityId:message.communityId,graph:${JSON.stringify(detail)}},"*"),0);
  } else if(message.type==="compare") {
    setTimeout(()=>window.postMessage({
      type:"comparison",
      commit:message.commit,
      parent:message.parent,
      realization:"r-"+message.commit.slice(0,1),
      fingerprint:"f-"+message.commit.slice(0,1),
      currentGraph:window.historyGraphs[message.commit],
      parentGraph:window.historyGraphs[message.parent],
      semanticDiff:{findings:[{summary:"Fixture comparison"}]}
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
          window.postMessage({type:"timeline",repositoryId:"fixture",timeline:window.fixtureTimeline},"*");
          window.postMessage({type:"buildSucceeded",commit:message.commit},"*");
        },220);
      }
    }
  } else if(message.type==="openSource") {
    window.openedSource=message.source;
  }
}})</script><script src="/history.js"></script></body></html>`;
}

function harness(script: string, hydration: unknown): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass ${script} fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>window.acquireVsCodeApi=()=>({postMessage(message){if(message.type==="ready")setTimeout(()=>window.postMessage(${JSON.stringify(hydration)},"*"),0)}})</script><script src="/${script}.js"></script></body></html>`;
}
