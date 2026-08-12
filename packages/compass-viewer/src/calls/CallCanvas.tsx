import { CompassGraph, type GraphHost } from "../graph/CompassGraph";
import type { GraphViewModel } from "../contracts/graph";
import type { CallGraphResponse } from "../contracts/callGraph";

const resolutionCommunity = {
  resolved: 0,
  inferred: 1,
  ambiguous: 2,
  unresolved: 3
} as const;

export function CallCanvas({
  graph,
  host
}: {
  graph: CallGraphResponse;
  host: GraphHost;
}) {
  const nodeResolution = new Map<string, keyof typeof resolutionCommunity>();
  const depths = callDepths(graph);
  for (const edge of graph.edges) {
    const current = nodeResolution.get(edge.target);
    if (!current || resolutionCommunity[edge.resolution] > resolutionCommunity[current]) {
      nodeResolution.set(edge.target, edge.resolution);
    }
  }
  const model: GraphViewModel = {
    schema: "compass.viewer.graph/1",
    title: `Calls from ${graph.nodes.find((node) => node.id === graph.rootSymbol)?.name ?? graph.rootSymbol}`,
    stats: {
      nodes: graph.nodes.length,
      edges: graph.edges.length,
      communities: 4,
      aggregated: false
    },
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      label: node.name,
      kind: node.unresolved ? "Unresolved call" : "Function",
      community: resolutionCommunity[nodeResolution.get(node.id) ?? "resolved"],
      communityName: nodeResolution.get(node.id) ?? "resolved",
      depth: depths.get(node.id),
      root: node.id === graph.rootSymbol,
      source: sourceLocation(node)
    })),
    edges: graph.edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      relation: `${edge.callee} · ${edge.resolution}`,
      confidence: edge.resolution === "resolved" ? "extracted"
        : edge.resolution === "inferred" ? "inferred" : "ambiguous"
    })),
    communities: [
      { id: 0, label: "Resolved", color: "#59A14F", hidden: false },
      { id: 1, label: "Inferred", color: "#4E79A7", hidden: false },
      { id: 2, label: "Ambiguous", color: "#F28E2B", hidden: false },
      { id: 3, label: "Unresolved", color: "#E15759", hidden: false }
    ],
    hyperedges: []
  };
  return <CompassGraph model={model} host={host} preferredLayout="hierarchical" />;
}

function callDepths(graph: CallGraphResponse): Map<string, number> {
  const neighbors = new Map<string, string[]>();
  for (const edge of graph.edges) {
    const source = neighbors.get(edge.source) ?? [];
    source.push(edge.target);
    neighbors.set(edge.source, source);
    const target = neighbors.get(edge.target) ?? [];
    target.push(edge.source);
    neighbors.set(edge.target, target);
  }
  const depths = new Map([[graph.rootSymbol, 0]]);
  const queue = [graph.rootSymbol];
  while (queue.length > 0) {
    const current = queue.shift();
    if (current === undefined) break;
    const depth = depths.get(current) ?? 0;
    for (const neighbor of neighbors.get(current) ?? []) {
      if (depths.has(neighbor)) continue;
      depths.set(neighbor, depth + 1);
      queue.push(neighbor);
    }
  }
  return depths;
}

function sourceLocation(
  node: CallGraphResponse["nodes"][number]
): GraphViewModel["nodes"][number]["source"] {
  if (node.anchor) {
    return {
      file: node.anchor.source_file,
      startByte: node.anchor.start_byte,
      endByte: node.anchor.end_byte
    };
  }
  if (!node.file) return undefined;
  return {
    file: node.file,
    ...(node.startLine != null ? { startLine: node.startLine } : {}),
    ...(node.endLine != null ? { endLine: node.endLine } : {}),
    ...(node.startByte != null ? { startByte: node.startByte } : {}),
    ...(node.endByte != null ? { endByte: node.endByte } : {})
  };
}
