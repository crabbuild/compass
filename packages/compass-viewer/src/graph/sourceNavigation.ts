import type { GraphEdge, GraphNode, SourceLocation } from "../contracts/graph";

export function navigableSource(node: GraphNode): SourceLocation | undefined {
  return navigableLocation(node.source);
}

export function navigableRelationshipSource(edge: GraphEdge): SourceLocation | undefined {
  return navigableLocation(edge.relationshipSite);
}

function navigableLocation(source: SourceLocation | undefined): SourceLocation | undefined {
  if (!source?.file.trim()) return undefined;
  const located = source.startLine !== undefined
    || source.endLine !== undefined
    || source.startByte !== undefined
    || source.endByte !== undefined;
  return located ? source : undefined;
}
