import type {
  CodeQueryEdge,
  CodeQueryNode,
  CodeQueryResponse
} from "@compass/viewer/contracts/codeQuery";
import type {
  GraphEdge,
  GraphNode,
  GraphViewModel
} from "@compass/viewer/contracts/graph";

const QUERY_COMMUNITY = 0;

export function codeQueryGraphViewModel(
  result: CodeQueryResponse,
  title: string
): GraphViewModel {
  const nodeIds = new Set(result.nodes.map((node) => node.id));
  const edges = result.edges
    .filter((edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target))
    .map(graphEdge);
  const degrees = new Map<string, number>();
  for (const edge of edges) {
    degrees.set(edge.source, (degrees.get(edge.source) ?? 0) + 1);
    degrees.set(edge.target, (degrees.get(edge.target) ?? 0) + 1);
  }
  const nodes = result.nodes.map((node) => graphNode(node, degrees.get(node.id) ?? 0));
  return {
    schema: "compass.viewer.graph/1",
    title,
    stats: {
      nodes: nodes.length,
      edges: edges.length,
      communities: nodes.length > 0 ? 1 : 0,
      aggregated: false
    },
    nodes,
    edges,
    communities: nodes.length > 0
      ? [{ id: QUERY_COMMUNITY, label: "Query result", color: "#6f7bf7", hidden: false }]
      : [],
    hyperedges: []
  };
}

function graphNode(node: CodeQueryNode, degree: number): GraphNode {
  const signature = node.details?.type === "symbol"
    ? node.details.data.signature ?? undefined
    : undefined;
  return {
    id: node.id,
    label: node.name,
    kind: node.kind,
    community: QUERY_COMMUNITY,
    communityName: "Query result",
    degree,
    ...(node.language ? { language: node.language } : {}),
    ...(signature ? { signature } : {}),
    ...(node.source ? { source: node.source } : {}),
    codeEvidence: node.evidence
  };
}

function graphEdge(edge: CodeQueryEdge): GraphEdge {
  const edgeConfidence = confidence(edge);
  return {
    id: edge.id,
    source: edge.source,
    target: edge.target,
    relation: edge.kind,
    ...(edge.relationshipSite ? { relationshipSite: edge.relationshipSite } : {}),
    ...(edge.details ? { details: edge.details } : {}),
    ...(edgeConfidence ? { confidence: edgeConfidence } : {}),
    codeEvidence: edge.evidence
  };
}

function confidence(
  edge: CodeQueryEdge
): GraphEdge["confidence"] {
  if (edge.evidence.some((item) => item.confidence === "ambiguous")) return "ambiguous";
  if (edge.evidence.some((item) => item.confidence === "inferred")) return "inferred";
  return edge.evidence.length > 0 ? "extracted" : undefined;
}
