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
      source: node.anchor ? {
        file: node.anchor.source_file,
        startByte: node.anchor.start_byte,
        endByte: node.anchor.end_byte
      } : undefined
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
  return <CompassGraph model={model} host={host} />;
}
