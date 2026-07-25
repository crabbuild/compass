import type { GraphNode, GraphViewModel, SourceLocation } from "../contracts/graph";
import { navigableSource } from "./sourceNavigation";

export type GraphNodeActivation =
  | { type: "community"; communityId: number }
  | { type: "source"; source: SourceLocation }
  | { type: "none" };

export function graphNodeActivation(
  model: GraphViewModel,
  node: GraphNode,
  detailCommunityId?: number
): GraphNodeActivation {
  if (detailCommunityId === undefined
    && model.stats.aggregated
    && node.memberCount !== undefined) {
    return { type: "community", communityId: node.community };
  }
  const source = navigableSource(node);
  return source ? { type: "source", source } : { type: "none" };
}
