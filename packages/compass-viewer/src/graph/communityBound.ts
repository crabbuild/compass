import type { GraphViewModel } from "../contracts/graph";
import type { CommunityGraphDetail } from "./CompassGraph";

/**
 * Bound of an embedded community detail that holds fewer symbols than the
 * community itself.
 *
 * A standalone document can only publish a window of a community that exceeds
 * its export budget. Its overview still carries the community's exact member
 * count, so the viewer can name the window instead of presenting the most
 * connected symbols as the whole community.
 */
export function embeddedCommunityBound(
  overview: GraphViewModel,
  communityId: number,
  detail: GraphViewModel | undefined
): CommunityGraphDetail["bounded"] {
  if (!detail) return undefined;
  const parentMembers = overview.nodes.find(
    (node) => node.community === communityId && node.memberCount !== undefined
  )?.memberCount;
  if (parentMembers === undefined || detail.nodes.length >= parentMembers) return undefined;
  return {
    limit: detail.nodes.length,
    parentMembers,
    currentMembers: detail.nodes.length,
    scope: "export"
  };
}
