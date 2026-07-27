import type { GraphEdge } from "../contracts/graph";

export type EdgeLabelVisibility = {
  forceLabels: boolean;
  focusedNodeId: string | null;
  focusedEdgeId: string | null;
  hoveredEdgeId: string | null;
};

export function formatGraphEdgeLabel(
  edge: Pick<GraphEdge, "relation" | "confidence">
): string {
  const relation = edge.relation.trim();
  const confidence = edge.confidence?.trim().toLocaleUpperCase();
  if (relation && confidence) return `${relation} [${confidence}]`;
  if (relation) return relation;
  return confidence ? `[${confidence}]` : "";
}

export function shouldShowGraphEdgeLabel(
  edge: Pick<GraphEdge, "id" | "source" | "target">,
  visibility: EdgeLabelVisibility
): boolean {
  return visibility.forceLabels
    || visibility.hoveredEdgeId === edge.id
    || visibility.focusedEdgeId === edge.id
    || visibility.focusedNodeId === edge.source
    || visibility.focusedNodeId === edge.target;
}
