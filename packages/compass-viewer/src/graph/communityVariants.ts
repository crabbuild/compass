import type { GraphViewModel } from "../contracts/graph";
import {
  communityImportanceScore,
  communityNodeId,
  type CommunityOverview
} from "./communityOverview";
import { edgeSemanticCategory, type EdgeSemanticCategory } from "./semanticAppearance";

/**
 * One community as every overview variant sees it: presentation-ready facts,
 * no viewer-model fields, so the matrix, treemap, and lane designs can each
 * choose what to encode without touching `compass.viewer.graph/1`.
 */
export type CommunityVariantCommunity = {
  id: number;
  nodeId: string;
  label: string;
  color: string;
  memberCount: number;
  degree: number;
  importance: number;
};

export type CommunityVariantLink = {
  id: string;
  source: number;
  target: number;
  weight: number;
  relation: string;
  category: EdgeSemanticCategory;
};

export type CommunityVariantData = {
  /** Communities ordered by reader importance, highest first. */
  communities: readonly CommunityVariantCommunity[];
  byId: ReadonlyMap<number, CommunityVariantCommunity>;
  links: readonly CommunityVariantLink[];
  /** Total symbols the overview stands for, for honest coverage statements. */
  symbols: number;
};

/**
 * Variant data for either a viewer-derived overview or an overview an exporter
 * already aggregated. Relationship categories are only known for the derived
 * case; an exported aggregate keeps its own relation text and falls back to the
 * neutral category rather than guessing kinds.
 */
export function communityVariantData(
  model: GraphViewModel,
  overview?: CommunityOverview | undefined
): CommunityVariantData | undefined {
  const nodes = overview?.model.nodes ?? (model.stats.aggregated ? model.nodes : undefined);
  if (!nodes || nodes.length === 0) return undefined;
  const edges = overview?.model.edges ?? model.edges;
  const communityOfNode = new Map(nodes.map((node) => [node.id, node.community]));
  const importance = new Map<number, number>();
  for (const node of nodes) {
    importance.set(
      node.community,
      overview?.importance.get(node.id)
        ?? communityImportanceScore(node.memberCount ?? 1, node.degree ?? 0, 0)
    );
  }
  const order = overview?.communityOrder
    ?? [...nodes]
      .sort((left, right) =>
        (importance.get(right.community) ?? 0) - (importance.get(left.community) ?? 0)
        || (right.memberCount ?? 0) - (left.memberCount ?? 0)
        || left.id.localeCompare(right.id))
      .map((node) => node.community);
  const orderIndex = new Map(order.map((communityId, index) => [communityId, index]));
  const communities = [...nodes]
    .sort((left, right) =>
      (orderIndex.get(left.community) ?? 0) - (orderIndex.get(right.community) ?? 0)
      || left.id.localeCompare(right.id))
    .map((node) => ({
      id: node.community,
      nodeId: node.id,
      label: node.communityName?.trim() || node.label,
      color: node.color?.background ?? "#6688aa",
      memberCount: node.memberCount ?? 1,
      degree: node.degree ?? 0,
      importance: importance.get(node.community) ?? 0
    }));
  const communitiesById = new Map(communities.map((community) => [community.id, community]));
  const links: CommunityVariantLink[] = [];
  for (const edge of edges) {
    const source = communityOfNode.get(edge.source);
    const target = communityOfNode.get(edge.target);
    if (source === undefined || target === undefined || source === target) continue;
    if (!communitiesById.has(source) || !communitiesById.has(target)) continue;
    links.push({
      id: edge.id,
      source,
      target,
      weight: Math.max(1, edge.weight ?? 1),
      relation: edge.relation,
      category: overview?.edgeCategories.get(edge.id)
        ?? edgeSemanticCategory(edge.relation)
    });
  }
  return {
    communities,
    byId: communitiesById,
    links,
    symbols: communities.reduce((sum, community) => sum + community.memberCount, 0)
  };
}

/** Community ids the matrix shows, highest importance first. */
export function communityMatrixSelection(
  data: CommunityVariantData,
  limit: number
): { shown: readonly CommunityVariantCommunity[]; omitted: number } {
  const bounded = Math.max(1, Math.trunc(limit));
  return {
    shown: data.communities.slice(0, bounded),
    omitted: Math.max(0, data.communities.length - bounded)
  };
}

/**
 * Community pairs for the matrix. Every unordered pair appears once in each
 * direction so a reader can scan a row for "what this community talks to" and a
 * column for "what talks to it", which is the same relationship by construction.
 */
export function communityMatrixCells(
  data: CommunityVariantData,
  limit: number
): Array<{
  row: CommunityVariantCommunity;
  column: CommunityVariantCommunity;
  link: CommunityVariantLink | undefined;
}> {
  const { shown } = communityMatrixSelection(data, limit);
  const links = new Map<string, CommunityVariantLink>();
  for (const link of data.links) {
    links.set(`${link.source}:${link.target}`, link);
    links.set(`${link.target}:${link.source}`, link);
  }
  const cells: Array<{
    row: CommunityVariantCommunity;
    column: CommunityVariantCommunity;
    link: CommunityVariantLink | undefined;
  }> = [];
  for (const row of shown) {
    for (const column of shown) {
      cells.push({
        row,
        column,
        link: row.id === column.id ? undefined : links.get(`${row.id}:${column.id}`)
      });
    }
  }
  return cells;
}

export type TreemapTile = {
  community: CommunityVariantCommunity;
  x: number;
  y: number;
  width: number;
  height: number;
};

export type TreemapLayout = {
  tiles: readonly TreemapTile[];
  /** Communities folded into one disclosed tail tile. */
  omitted: { communities: number; symbols: number };
};

export const TREEMAP_VIEWBOX = { width: 1_000, height: 620 } as const;
export const TREEMAP_COMMUNITY_LIMIT = 240;

/**
 * Strip treemap: rows carry equal item counts, row height is proportional to the
 * row's total, and item width is proportional to the item's value. Area is
 * therefore exactly proportional to symbols for every tile, including the long
 * tail, which is what makes this variant honest about a repository with
 * thousands of communities.
 */
export function layoutCommunityTreemap(
  data: CommunityVariantData,
  limit = TREEMAP_COMMUNITY_LIMIT,
  viewBox: { width: number; height: number } = TREEMAP_VIEWBOX
): TreemapLayout {
  const bounded = Math.max(1, Math.trunc(limit));
  const shown = data.communities.slice(0, bounded);
  const hidden = data.communities.slice(bounded);
  const omitted = {
    communities: hidden.length,
    symbols: hidden.reduce((sum, community) => sum + community.memberCount, 0)
  };
  // A disclosed tail tile keeps the area honest without pretending to be a
  // community: it is labelled by count and never claims a group identity.
  const entries: Array<{ community: CommunityVariantCommunity; value: number; tail: boolean }> = [
    ...shown.map((community) => ({ community, value: Math.max(1, community.memberCount), tail: false })),
    ...(omitted.communities > 0
      ? [{
        community: {
          id: -1,
          nodeId: "community:tail",
          label: `${omitted.communities.toLocaleString()} more communities`,
          color: "#8A93A0",
          memberCount: omitted.symbols,
          degree: 0,
          importance: 0
        },
        value: Math.max(1, omitted.symbols),
        tail: true
      }]
      : [])
  ];
  return { tiles: stripLayout(entries, viewBox), omitted };
}

type StripEntry = {
  community: CommunityVariantCommunity;
  value: number;
  tail: boolean;
};

function stripLayout(
  entries: ReadonlyArray<StripEntry>,
  viewBox: { width: number; height: number }
): TreemapTile[] {
  const total = entries.reduce((sum, entry) => sum + entry.value, 0);
  if (total <= 0) return [];
  const rows = Math.max(
    1,
    Math.min(entries.length, Math.round(Math.sqrt(entries.length * viewBox.height / viewBox.width)))
  );
  const grouped: StripEntry[][] = [];
  let remaining: StripEntry[] = [...entries];
  let remainingTotal = total;
  for (let row = 0; row < rows && remaining.length > 0; row += 1) {
    const rowsLeft = rows - row;
    const target = remainingTotal / rowsLeft;
    const group: StripEntry[] = [];
    let sum = 0;
    while (remaining.length > 0) {
      const next = remaining[0]!;
      const shouldStop = group.length > 0
        && sum + next.value / 2 > target
        && remaining.length > rowsLeft - 1;
      if (shouldStop) break;
      group.push(next);
      sum += next.value;
      remaining = remaining.slice(1);
    }
    remainingTotal -= sum;
    grouped.push(group);
  }
  const tiles: TreemapTile[] = [];
  let y = 0;
  for (const group of grouped) {
    const sum = group.reduce((totalValue, entry) => totalValue + entry.value, 0);
    const height = viewBox.height * (sum / total);
    let x = 0;
    for (const entry of group) {
      const width = viewBox.width * (entry.value / sum);
      tiles.push({
        community: entry.community,
        x,
        y,
        width,
        height
      });
      x += width;
    }
    y += height;
  }
  return tiles;
}

export type LaneEntry = {
  community: CommunityVariantCommunity;
  x: number;
  width: number;
};

export type LaneLayout = {
  lanes: ReadonlyArray<{ tier: number; entries: readonly LaneEntry[] }>;
  omitted: { communities: number; symbols: number };
};

export const LANE_LIMIT = 60;
export const LANE_TIER_SIZE = 12;

/**
 * Importance tiers: the most coupled and boundary-rich communities lead the
 * first lane, and width inside a lane is proportional to symbols. The remaining
 * communities are disclosed as one tail bar per the same rule as the treemap.
 */
export function layoutCommunityLanes(
  data: CommunityVariantData,
  limit = LANE_LIMIT,
  tierSize = LANE_TIER_SIZE
): LaneLayout {
  const bounded = Math.max(1, Math.trunc(limit));
  const perTier = Math.max(1, Math.trunc(tierSize));
  const shown = data.communities.slice(0, bounded);
  const hidden = data.communities.slice(bounded);
  const lanes: Array<{ tier: number; entries: LaneEntry[] }> = [];
  for (let index = 0; index < shown.length; index += perTier) {
    const tierEntries = shown.slice(index, index + perTier);
    const total = tierEntries.reduce((sum, community) => sum + Math.max(1, community.memberCount), 0);
    let x = 0;
    const entries: LaneEntry[] = tierEntries.map((community) => {
      const width = Math.max(1, community.memberCount) / total;
      const entry = { community, x, width };
      x += width;
      return entry;
    });
    lanes.push({ tier: lanes.length, entries });
  }
  const omitted = {
    communities: hidden.length,
    symbols: hidden.reduce((sum, community) => sum + community.memberCount, 0)
  };
  if (omitted.communities > 0) {
    lanes.push({
      tier: lanes.length,
      entries: [{
        community: {
          id: -1,
          nodeId: "community:tail",
          label: `${omitted.communities.toLocaleString()} more communities`,
          color: "#8A93A0",
          memberCount: omitted.symbols,
          degree: 0,
          importance: 0
        },
        x: 0,
        width: 1
      }]
    });
  }
  return { lanes, omitted };
}

/**
 * Couplings between visible lane entries, bounded and ordered deterministically
 * so the ribbons stay readable and identical for identical input.
 */
export function laneRibbons(
  data: CommunityVariantData,
  layout: LaneLayout,
  limit = 240
): Array<{
  link: CommunityVariantLink;
  source: LaneEntry;
  target: LaneEntry;
  sourceTier: number;
  targetTier: number;
}> {
  const placed = new Map<number, { entry: LaneEntry; tier: number }>();
  for (const lane of layout.lanes) {
    for (const entry of lane.entries) {
      if (entry.community.id >= 0) placed.set(entry.community.id, { entry, tier: lane.tier });
    }
  }
  return [...data.links]
    .sort((left, right) =>
      right.weight - left.weight
      || left.source - right.source
      || left.target - right.target
      || left.id.localeCompare(right.id))
    .flatMap((link) => {
      const source = placed.get(link.source);
      const target = placed.get(link.target);
      if (!source || !target) return [];
      return [{
        link,
        source: source.entry,
        target: target.entry,
        sourceTier: source.tier,
        targetTier: target.tier
      }];
    })
    .slice(0, Math.max(1, Math.trunc(limit)));
}

export function communityVariantNodeId(communityId: number): string {
  return communityNodeId(communityId);
}
