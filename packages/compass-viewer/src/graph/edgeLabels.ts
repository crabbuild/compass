import type { GraphEdge } from "../contracts/graph";

export type EdgeLabelVisibility = {
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
  edge: Pick<GraphEdge, "id">,
  visibility: EdgeLabelVisibility
): boolean {
  return visibility.hoveredEdgeId === edge.id;
}
