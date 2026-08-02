import type { GraphEdge } from "../contracts/graph";

const RELATION_LABELS: Readonly<Record<string, string>> = {
  routes_to: "routes to",
  publishes: "publishes",
  subscribes: "subscribes",
  produces: "produces",
  consumes: "consumes",
  schedules: "schedules",
  triggers: "triggers",
  handles: "handles",
  registers: "registers",
  reads: "reads",
  writes: "writes",
  maps_to: "maps to",
  depends_on: "depends on"
};

export type EdgeLabelVisibility = {
  hoveredEdgeId: string | null;
};

export function formatGraphEdgeLabel(
  edge: Pick<GraphEdge, "relation" | "confidence" | "details" | "relationshipSite">
): string {
  const relation = edge.relation.trim();
  const relationLabel = RELATION_LABELS[relation] ?? relation;
  const route = edge.details?.type === "route" ? edge.details.data : undefined;
  const routeStage = route
    ? `${route.stage}${route.position === null || route.position === undefined
      ? ""
      : ` ${route.position + 1}`}`
    : undefined;
  const semanticLabel = [relationLabel, routeStage, route?.operation]
    .filter((part) => part !== undefined && part !== "")
    .join(" · ");
  const confidence = edge.confidence?.trim().toLocaleUpperCase();
  const relationship = semanticLabel && confidence
    ? `${semanticLabel} [${confidence}]`
    : semanticLabel || (confidence ? `[${confidence}]` : "");
  const source = formatRelationshipSite(edge.relationshipSite);
  return [relationship, source].filter(Boolean).join(" · ");
}

function formatRelationshipSite(site: GraphEdge["relationshipSite"]): string {
  if (!site?.file.trim()) return "";
  const startLine = site.startLine;
  const endLine = site.endLine;
  if (startLine !== undefined) {
    const lines = endLine !== undefined && endLine !== startLine
      ? `${startLine}–${endLine}`
      : String(startLine);
    return `${site.file}:${lines}`;
  }
  if (site.startByte !== undefined) return `${site.file}:byte ${site.startByte}`;
  return site.file;
}

export function shouldShowGraphEdgeLabel(
  edge: Pick<GraphEdge, "id">,
  visibility: EdgeLabelVisibility
): boolean {
  return visibility.hoveredEdgeId === edge.id;
}
