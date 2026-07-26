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
  const architectureNodes = Array.from({ length: 31 }, (_, index) => ({
    id: index === 0 ? "authenticate" : index === 1 ? "database" : `symbol-${index}`,
    label: index === 0 ? "authenticate" : index === 1 ? "database" : `symbol${index}`,
    kind: index % 5 === 0 ? "class" : "function",
    sourceFile: `src/api/module-${index}.ts`
  }));
  const architectureEdges = Array.from({ length: 53 }, (_, index) => ({
    source: "authenticate",
    target: index % 4 === 0 ? "database" : `symbol-${2 + (index % 29)}`,
    relation: index % 3 === 0 ? "calls" : "uses",
    confidence: index % 5 === 0 ? "inferred" : "extracted"
  }));
  const architectureSections = Array.from({ length: 26 }, (_, index) => ({
    id: `section-${index}`,
    name: index === 0 ? "API" : index === 1 ? "Storage" : `Section ${index}`,
    communities: [`${index}`],
    nodes: index === 0
      ? architectureNodes
      : index === 1
        ? [{
          id: "database-adapter",
          label: "database adapter",
          kind: "class",
          sourceFile: "src/storage/database.ts"
        }]
        : [],
    edges: index === 0 ? architectureEdges : []
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
    statistics: { nodes: 32, edges: 53, communities: 26, hyperedges: 0, extracted: 42, inferred: 11, ambiguous: 0 },
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
      communityDetail
    )
  );
  await writeFile(path.join(output, "query.html"), queryHarness());
  await writeFile(path.join(output, "initialize.html"), initializationHarness());
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
    if(new URLSearchParams(window.location.search).has("large") && message.type==="ready") {
      setTimeout(()=>window.postMessage({
        type:"graphLoadStatus",
        mode:"large",
        graphBytes:44275915,
        phase:"exporting"
      },"*"),0);
    }
    if(new URLSearchParams(window.location.search).has("error") && (message.type==="ready" || message.type==="retry")) {
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
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass architecture fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.architectureHostMessages=[];
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
  if(message.type!=="ready" && message.type!=="retry") return;
  const params=new URLSearchParams(window.location.search);
  if(params.has("error")) {
    setTimeout(()=>window.postMessage({type:"error",message:"Architecture export failed"},"*"),20);
    return;
  }
  const delay=params.has("delay") ? 800 : 0;
  setTimeout(()=>window.postMessage({
    type:"hydrate",
    repositoryId:"fixture",
    model:${JSON.stringify(model)}
  },"*"),delay);
}})</script><script src="/architecture.js"></script></body></html>`;
}

function queryHarness(): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Compass query fixture</title><link rel="stylesheet" href="/viewer.css"></head><body><div id="root"></div><script>
window.queryHostMessages=[];
window.queryTimer=undefined;
window.acquireVsCodeApi=()=>({postMessage(message){
  window.queryHostMessages.push(message);
  if(message.type==="openSource") {
    window.openedQuerySource=message.source;
    return;
  }
  if(message.type==="openGraph") {
    window.openedQueryGraph=true;
    return;
  }
  if(message.type==="ready") {
    setTimeout(()=>window.postMessage({type:"state",running:false},"*"),0);
    return;
  }
  if(message.type==="cancel") {
    clearTimeout(window.queryTimer);
    window.postMessage({type:"state",running:false},"*");
    return;
  }
  if(message.type!=="execute") return;
  window.postMessage({type:"state",running:true},"*");
  const params=new URLSearchParams(window.location.search);
  const delay=params.has("delay") ? 1200 : 20;
  window.queryTimer=setTimeout(()=>{
    if(params.has("error")) {
      window.postMessage({type:"error",message:"CompassQL could not parse this query"},"*");
    } else if(params.get("result")==="rows") {
      window.postMessage({
        type:"result",
        result:{
          mode:message.request.mode,
          json:{rows:[{symbol:"run",calls:3},{symbol:"save",calls:2}]},
          durationMs:18
        }
      },"*");
    } else if(params.get("result")==="traversal") {
      window.postMessage({
        type:"result",
        result:{
          mode:message.request.mode,
          text:"Traversal: BFS depth=2 | Start: ['Pipeline'] | 146 nodes found\\n\\nNODE Pipeline [src=caching/util/src/Pipeline.scala loc=L154 community=Pipeline]\\nNODE .assert() [src=caching/util/src/AssertMacros.scala loc=L32 community=.iassert]\\nNODE String [src= loc= community=EtcdClient]",
          durationMs:24
        }
      },"*");
    } else {
      window.postMessage({
        type:"result",
        result:{
          mode:message.request.mode,
          text:"Authentication reaches storage through the repository service.",
          durationMs:24
        }
      },"*");
    }
  },delay);
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
  window.initializationTimers.push(setTimeout(
    ()=>window.postMessage({type:"succeeded",message:"compass is indexed and ready for graph exploration."},"*"),
    1240
  ));
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
window.historyBootstrapAttempts=0;
window.historyGeneration=0;
window.emitHistoryMessage=(message)=>window.postMessage(message,"*");
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
    const delay=loadScenario==="slow" ? 500 : message.commit.startsWith("a") ? 180 : 120;
    if(loadScenario==="error") {
      setTimeout(()=>window.postMessage({
        type:"error",
        operation:"Load graph",
        commit:message.commit,
        message:"Fixture graph load failed"
      },"*"),delay);
    } else {
      setTimeout(()=>window.postMessage({
        type:"graph",
        commit:message.commit,
        realization:"r-"+message.commit.slice(0,1),
        fingerprint:"f-"+message.commit.slice(0,1),
        graph:window.historyGraphs[message.commit]
      },"*"),delay);
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
