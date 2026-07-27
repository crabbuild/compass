import type { CallGraphResponse } from "../contracts/callGraph";

export function mergeExpansion(
  current: CallGraphResponse,
  expansion: CallGraphResponse
): CallGraphResponse {
  const nodes = new Map(current.nodes.map((node) => [node.id, node]));
  for (const node of expansion.nodes) nodes.set(node.id, node);
  const edges = new Map(current.edges.map((edge) => [edge.id, edge]));
  for (const edge of expansion.edges) {
    const existing = edges.get(edge.id);
    edges.set(edge.id, existing ? {
      ...edge,
      callSites: uniqueSites([...existing.callSites, ...edge.callSites])
    } : edge);
  }
  const continuations = new Map(
    [...current.continuations, ...expansion.continuations]
      .map((continuation) => [
        `${continuation.symbol}:${continuation.direction}:${continuation.nextDepth}`,
        continuation
      ])
  );
  return {
    ...current,
    nodes: [...nodes.values()],
    edges: [...edges.values()],
    continuations: [...continuations.values()],
    truncated: current.truncated || expansion.truncated,
    coverage: {
      ...expansion.coverage,
      resolved: count(edges, "resolved"),
      inferred: count(edges, "inferred"),
      ambiguous: count(edges, "ambiguous"),
      unresolved: count(edges, "unresolved")
    }
  };
}

function uniqueSites<T extends CallGraphResponse["edges"][number]["callSites"][number]>(
  sites: T[]
): T[] {
  return [...new Map(sites.map((site) => [
    "anchor" in site
      ? `${site.anchor.source_file}:${site.anchor.start_byte}:${site.anchor.end_byte}`
      : `${site.sourceFile}:${site.line}:${site.startByte}:${site.endByte}`,
    site
  ])).values()];
}

function count(
  edges: Map<string, CallGraphResponse["edges"][number]>,
  resolution: string
): number {
  return [...edges.values()].filter((edge) => edge.resolution === resolution).length;
}
