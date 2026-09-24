import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";
import {
  EDGE_SEMANTIC_CATEGORIES,
  edgeSemanticCategory,
  isBoundaryKind,
  type EdgeSemanticCategory
} from "./semanticAppearance";

/**
 * Node count above which a flat symbol canvas stops being readable: at that
 * size one screen cannot show both the topology and the symbols. The viewer
 * opens such graphs on an aggregated community overview and lets the reader
 * open one community at a time.
 */
export const COMMUNITY_OVERVIEW_NODE_THRESHOLD = 600;

/** Below this many distinct communities an aggregation hides more than it shows. */
export const COMMUNITY_OVERVIEW_MINIMUM_COMMUNITIES = 3;

/** Upper bound on symbols opened with one community so a drill-down stays bounded. */
export const COMMUNITY_DETAIL_NODE_BUDGET = 2_000;

const MINIMUM_COMMUNITY_NODE_SIZE = 9;
const MAXIMUM_COMMUNITY_NODE_SIZE = 26;

/** Font size, in canvas units, of a community bubble label. */
export const COMMUNITY_LABEL_FONT_SIZE = 13;

/** Characters a community bubble label keeps before it is truncated for the canvas. */
export const COMMUNITY_LABEL_MAXIMUM_CHARACTERS = 18;

export type CommunityMemberIndex = {
  /** Deterministically ordered member nodes for every community in the model. */
  members: ReadonlyMap<number, readonly GraphNode[]>;
  /** Community ids ordered by size, then connectivity, then id. */
  order: readonly number[];
};

export type CommunityDetailView = {
  model: GraphViewModel;
  bounded?: {
    limit: number;
    parentMembers: number;
    currentMembers: number;
  } | undefined;
};

export type CommunityOverview = {
  model: GraphViewModel;
  /** Dominant relationship category for every aggregated community edge. */
  edgeCategories: ReadonlyMap<string, EdgeSemanticCategory>;
  /** Importance score per community bubble node id. */
  importance: ReadonlyMap<string, number>;
  /** Community ids ordered by importance, for panel and list ordering. */
  communityOrder: readonly number[];
};

export function communityNodeId(communityId: number): string {
  return `community:${communityId}`;
}

/**
 * A repository overview only replaces the symbol canvas when the model is not
 * already an overview, is large enough that one screen cannot stay readable,
 * and actually splits into several communities.
 */
export function communityOverviewApplies(model: GraphViewModel): boolean {
  if (model.stats.aggregated) return false;
  if (model.nodes.length < COMMUNITY_OVERVIEW_NODE_THRESHOLD) return false;
  const communities = new Set(model.nodes.map((node) => node.community));
  return communities.size >= COMMUNITY_OVERVIEW_MINIMUM_COMMUNITIES
    && communities.size < model.nodes.length;
}

/**
 * Members stay ordered by connectivity so the most important symbols lead both
 * the drill-down and any bounded detail, and so equal inputs always produce the
 * same view.
 */
export function communityMemberIndex(model: GraphViewModel): CommunityMemberIndex {
  const grouped = new Map<number, GraphNode[]>();
  for (const node of model.nodes) {
    const group = grouped.get(node.community) ?? [];
    group.push(node);
    grouped.set(node.community, group);
  }
  const members = new Map<number, readonly GraphNode[]>();
  const order = [...grouped.keys()].sort((left, right) => {
    const leftMembers = grouped.get(left) ?? [];
    const rightMembers = grouped.get(right) ?? [];
    return rightMembers.length - leftMembers.length
      || left - right;
  });
  for (const communityId of order) {
    const group = grouped.get(communityId) ?? [];
    members.set(communityId, [...group].sort((left, right) =>
      (right.degree ?? 0) - (left.degree ?? 0)
      || (right.memberCount ?? 0) - (left.memberCount ?? 0)
      || left.id.localeCompare(right.id)));
  }
  return { members, order };
}

export function communityNodeSize(memberCount: number, maximumMembers: number): number {
  const largest = Math.max(1, maximumMembers);
  const share = Math.sqrt(Math.max(1, memberCount) / largest);
  return MINIMUM_COMMUNITY_NODE_SIZE
    + (MAXIMUM_COMMUNITY_NODE_SIZE - MINIMUM_COMMUNITY_NODE_SIZE) * share;
}

/**
 * How much a community deserves the reader's attention: its size, the number of
 * communities it couples to, and whether it holds boundary entities such as
 * routes, jobs, or database objects. Sub-linear in every term so one huge
 * community cannot crowd out every label.
 */
export function communityImportanceScore(
  memberCount: number,
  interCommunityDegree: number,
  boundaryMemberCount: number
): number {
  return Math.log2(1 + Math.max(0, memberCount))
    + 1.5 * Math.log2(1 + Math.max(0, interCommunityDegree))
    + 2 * Math.log2(1 + Math.max(0, boundaryMemberCount));
}

/**
 * How many community bubbles may carry a canvas label. A fitted overview can
 * only hold a bounded number of readable labels, so the largest communities
 * keep theirs and every other bubble stays a hover and inspector target.
 */
export function communityOverviewLabelLimit(communityCount: number): number {
  if (communityCount <= 40) return communityCount;
  if (communityCount <= 120) return 20;
  if (communityCount <= 400) return 14;
  return 10;
}

/**
 * Bubble ids that carry a label in the community overview. When the caller has
 * importance scores (the derived overview computes them from member counts,
 * coupling, and boundary content) they decide the ranking; otherwise size and
 * coupling stand in, so the same budget always yields the same set.
 */
export function communityOverviewLabelledIds(
  nodes: readonly GraphNode[],
  importance?: ReadonlyMap<string, number> | undefined
): ReadonlySet<string> {
  const limit = communityOverviewLabelLimit(nodes.length);
  if (limit >= nodes.length) return new Set(nodes.map((node) => node.id));
  const score = (node: GraphNode) => importance?.get(node.id)
    ?? communityImportanceScore(
      node.memberCount ?? 0,
      node.degree ?? 0,
      0
    );
  return new Set([...nodes]
    .sort((left, right) =>
      score(right) - score(left)
      || (right.memberCount ?? 0) - (left.memberCount ?? 0)
      || (right.degree ?? 0) - (left.degree ?? 0)
      || left.id.localeCompare(right.id))
    .slice(0, limit)
    .map((node) => node.id));
}

/**
 * Bounded canvas label for a community bubble. The full name stays available in
 * the hover card, the inspector, and the community list.
 */
export function communityOverviewLabelText(label: string): string {
  const collapsed = label.replace(/\s+/g, " ").trim();
  if (collapsed.length <= COMMUNITY_LABEL_MAXIMUM_CHARACTERS) return collapsed;
  return `${collapsed.slice(0, COMMUNITY_LABEL_MAXIMUM_CHARACTERS - 1).trimEnd()}…`;
}

/**
 * Aggregate one edge per unordered community pair, mirroring the aggregated
 * export: the weight is the exact number of relationships that crossed that
 * pair, and no direction or relationship kind is invented. The pair keeps the
 * exact count of every relationship verb so the overview can state what binds
 * two communities instead of one opaque total.
 */
function aggregateCommunityEdges(
  model: GraphViewModel,
  communityOf: ReadonlyMap<string, number>
): {
  edges: GraphEdge[];
  categories: Map<string, EdgeSemanticCategory>;
  pairs: ReadonlyArray<readonly [number, number]>;
} {
  const counts = new Map<string, {
    source: number;
    target: number;
    total: number;
    relations: Map<string, number>;
  }>();
  for (const edge of model.edges) {
    const source = communityOf.get(edge.source);
    const target = communityOf.get(edge.target);
    if (source === undefined || target === undefined || source === target) continue;
    const low = Math.min(source, target);
    const high = Math.max(source, target);
    const key = `${low}:${high}`;
    const pair = counts.get(key)
      ?? { source: low, target: high, total: 0, relations: new Map<string, number>() };
    pair.total += 1;
    pair.relations.set(edge.relation, (pair.relations.get(edge.relation) ?? 0) + 1);
    counts.set(key, pair);
  }
  const ordered = [...counts.entries()]
    .sort(([, left], [, right]) =>
      left.source - right.source
      || left.target - right.target);
  const categories = new Map<string, EdgeSemanticCategory>();
  return {
    edges: ordered.map(([key, { source, target, total, relations }]) => {
      const id = `community-edge:${key}`;
      const ranked = rankRelations(relations);
      const dominant = ranked[0];
      if (dominant) {
        categories.set(id, edgeSemanticCategory(dominant.relation));
      }
      return {
        id,
        source: communityNodeId(source),
        target: communityNodeId(target),
        relation: describeRelationMix(ranked, total),
        weight: total,
        confidence: "aggregated" as const
      };
    }),
    pairs: ordered.map(([, { source, target }]) => [source, target] as const),
    categories
  };
}

/**
 * Deterministic relationship ranking: exact count first, then the category
 * order the canvas palette already uses, then the raw verb.
 */
function rankRelations(
  relations: ReadonlyMap<string, number>
): Array<{ relation: string; count: number }> {
  return [...relations.entries()]
    .map(([relation, count]) => ({ relation, count }))
    .sort((left, right) =>
      right.count - left.count
      || categoryRank(left.relation) - categoryRank(right.relation)
      || left.relation.localeCompare(right.relation));
}

function categoryRank(relation: string): number {
  return EDGE_SEMANTIC_CATEGORIES.indexOf(edgeSemanticCategory(relation));
}

const RELATION_MIX_VISIBLE = 3;

/**
 * Exact, bounded relationship mix, for example `12 calls · 4 imports`.
 * Counts are never rounded and kinds are never merged.
 */
export function describeRelationMix(
  ranked: readonly { relation: string; count: number }[],
  total: number
): string {
  const visible = ranked.slice(0, RELATION_MIX_VISIBLE);
  const parts = visible.map(({ relation, count }) => `${count} ${relation}`);
  const omitted = total - visible.reduce((sum, entry) => sum + entry.count, 0);
  if (omitted > 0) parts.push(`${omitted} more`);
  return parts.join(" · ");
}

/**
 * Synthesize the community overview a reader sees first on a large repository:
 * one node per community with its member count, and one weighted edge per
 * community pair. The result carries `stats.aggregated`, so every existing
 * viewer affordance (community activation, bounded layouts, aggregated edge
 * styling) applies without a second code path. Relationship categories and
 * importance scores travel beside the model so the canvas can colour edges and
 * rank labels without adding fields to `compass.viewer.graph/1`.
 */
export function communityOverviewModel(model: GraphViewModel): CommunityOverview {
  const index = communityMemberIndex(model);
  const communityOf = new Map(model.nodes.map((node) => [node.id, node.community]));
  const nodeById = new Map(model.nodes.map((node) => [node.id, node]));
  const { edges, categories, pairs } = aggregateCommunityEdges(model, communityOf);
  const boundaryMembers = new Map<number, number>();
  for (const node of model.nodes) {
    if (!isBoundaryKind(node.kind)) continue;
    boundaryMembers.set(node.community, (boundaryMembers.get(node.community) ?? 0) + 1);
  }
  const neighborCommunities = new Map<number, Set<number>>();
  for (const [source, target] of pairs) {
    const sourceNeighbors = neighborCommunities.get(source) ?? new Set<number>();
    sourceNeighbors.add(target);
    neighborCommunities.set(source, sourceNeighbors);
    const targetNeighbors = neighborCommunities.get(target) ?? new Set<number>();
    targetNeighbors.add(source);
    neighborCommunities.set(target, targetNeighbors);
  }
  const maximumMembers = index.order.reduce(
    (largest, communityId) => Math.max(largest, index.members.get(communityId)?.length ?? 0),
    1
  );
  const importanceByCommunity = new Map<number, number>(index.order.map((communityId) => [
    communityId,
    communityImportanceScore(
      index.members.get(communityId)?.length ?? 0,
      neighborCommunities.get(communityId)?.size ?? 0,
      boundaryMembers.get(communityId) ?? 0
    )
  ]));
  const communityOrder = [...index.order].sort((left, right) =>
    (importanceByCommunity.get(right) ?? 0) - (importanceByCommunity.get(left) ?? 0)
    || (index.members.get(right)?.length ?? 0) - (index.members.get(left)?.length ?? 0)
    || left - right);
  const elementsByName = new Map(model.communities.map((community) => [community.id, community]));
  const nodes = communityOrder.map((communityId) => {
    const group = index.members.get(communityId) ?? [];
    const hub = group[0];
    const element = elementsByName.get(communityId);
    const declaredLabel = element?.label?.trim();
    const label = declaredLabel && declaredLabel !== `Community ${communityId}`
      ? declaredLabel
      : (hub?.communityName?.trim() || hub?.label?.trim() || `Community ${communityId}`);
    const color = element?.color ?? hub?.color?.background ?? "#7A838E";
    const language = hub ? nodeById.get(hub.id)?.language : undefined;
    return {
      id: communityNodeId(communityId),
      label,
      kind: "community",
      community: communityId,
      communityName: label,
      degree: neighborCommunities.get(communityId)?.size ?? 0,
      memberCount: group.length,
      detailAvailable: true,
      size: communityNodeSize(group.length, maximumMembers),
      color: { background: color, border: color },
      ...(language ? { language } : {})
    };
  });
  return {
    model: {
      ...model,
      stats: {
        nodes: nodes.length,
        edges: edges.length,
        communities: nodes.length,
        aggregated: true
      },
      nodes,
      edges,
      communities: model.communities.filter((community) =>
        index.members.has(community.id)),
      hyperedges: []
    },
    edgeCategories: categories,
    importance: new Map(nodes.map((node) => [
      node.id,
      importanceByCommunity.get(node.community) ?? 0
    ])),
    communityOrder
  };
}

/**
 * Bounded drill-down for one community. The detail keeps the exact member
 * records and the relationships whose endpoints are both inside the community;
 * nothing is synthesized except the bound.
 */
export function communityDetailModel(
  model: GraphViewModel,
  communityId: number,
  budget = COMMUNITY_DETAIL_NODE_BUDGET
): CommunityDetailView | undefined {
  const members = communityMemberIndex(model).members.get(communityId);
  if (!members || members.length === 0) return undefined;
  const limit = Math.max(1, Math.trunc(budget));
  const selected = members.slice(0, limit);
  const selectedIds = new Set(selected.map((node) => node.id));
  const edges = model.edges.filter((edge) =>
    selectedIds.has(edge.source) && selectedIds.has(edge.target));
  const element = model.communities.find((community) => community.id === communityId);
  return {
    model: {
      ...model,
      stats: {
        nodes: selected.length,
        edges: edges.length,
        communities: 1,
        aggregated: false
      },
      nodes: selected,
      edges,
      communities: element ? [element] : [],
      hyperedges: []
    },
    bounded: selected.length < members.length
      ? {
        limit: selected.length,
        parentMembers: members.length,
        currentMembers: selected.length
      }
      : undefined
  };
}
