import type { GraphViewModel } from "../contracts/graph";

export type GraphEdgeDirection = "both" | "outgoing" | "incoming";

export type GraphNeighborhood = {
  nodeIds: ReadonlySet<string>;
  edgeIds: ReadonlySet<string>;
};

export const MIN_NEIGHBORHOOD_DEPTH = 1;
export const MAX_NEIGHBORHOOD_DEPTH = 4;

export function clampNeighborhoodDepth(depth: number): number {
  return Math.max(
    MIN_NEIGHBORHOOD_DEPTH,
    Math.min(MAX_NEIGHBORHOOD_DEPTH, Math.trunc(depth))
  );
}

export function graphNeighborhood(
  model: GraphViewModel,
  rootNodeId: string,
  requestedDepth: number,
  direction: GraphEdgeDirection
): GraphNeighborhood {
  const depth = clampNeighborhoodDepth(requestedDepth);
  const outgoing = new Map<string, GraphViewModel["edges"]>();
  const incoming = new Map<string, GraphViewModel["edges"]>();
  for (const edge of model.edges) {
    const outgoingEdges = outgoing.get(edge.source) ?? [];
    outgoingEdges.push(edge);
    outgoing.set(edge.source, outgoingEdges);
    const incomingEdges = incoming.get(edge.target) ?? [];
    incomingEdges.push(edge);
    incoming.set(edge.target, incomingEdges);
  }
  for (const edges of [...outgoing.values(), ...incoming.values()]) {
    edges.sort((left, right) => left.id.localeCompare(right.id));
  }

  const nodeIds = new Set([rootNodeId]);
  const edgeIds = new Set<string>();
  let frontier = [rootNodeId];
  for (let level = 0; level < depth && frontier.length > 0; level += 1) {
    const next = new Set<string>();
    for (const nodeId of frontier.sort()) {
      if (direction !== "incoming") {
        for (const edge of outgoing.get(nodeId) ?? []) {
          edgeIds.add(edge.id);
          if (!nodeIds.has(edge.target)) next.add(edge.target);
        }
      }
      if (direction !== "outgoing") {
        for (const edge of incoming.get(nodeId) ?? []) {
          edgeIds.add(edge.id);
          if (!nodeIds.has(edge.source)) next.add(edge.source);
        }
      }
    }
    frontier = [...next];
    for (const nodeId of frontier) nodeIds.add(nodeId);
  }
  return { nodeIds, edgeIds };
}
