import type { GraphEdge } from "../contracts/graph";

export const EDGE_LABEL_ZOOM_THRESHOLD = 1.1;

export type EdgeLabelVisibility = {
  forceLabels: boolean;
  focusedNodeId: string | null;
  hoveredEdgeId: string | null;
  zoomScale: number;
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
    || visibility.focusedNodeId === edge.source
    || visibility.focusedNodeId === edge.target
    || visibility.zoomScale >= EDGE_LABEL_ZOOM_THRESHOLD;
}
