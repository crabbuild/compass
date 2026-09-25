import type { GraphNode, GraphViewModel } from "../contracts/graph";
import type {
  CommunityHierarchyView,
  HierarchyGroup,
  HierarchyLevel
} from "../contracts/hierarchy";

/** Couplings a community names before the panel bounds the list. */
export const COMMUNITY_COUPLING_LIMIT = 5;

/** Boundary kinds a community names before the list is bounded. */
export const COMMUNITY_BOUNDARY_LIMIT = 4;

const NO_COMMUNITY_NAMES: ReadonlyMap<number, string> = new Map();

/**
 * The name the published hierarchy gives every community: the finest level
 * holds one group per community, so its labels are the repository's own words
 * for them. An editor that published `labels.json` keeps its labels; a build
 * without them would otherwise show `Community 7` everywhere.
 */
export function hierarchyCommunityNames(
  hierarchy: CommunityHierarchyView | undefined
): ReadonlyMap<number, string> {
  if (!hierarchy) return NO_COMMUNITY_NAMES;
  const names = new Map<number, string>();
  const finest = hierarchy.levels[hierarchy.levels.length - 1];
  for (const group of finest?.groups ?? []) {
    if (group.community !== undefined) names.set(group.community, group.label);
  }
  return names.size > 0 ? names : NO_COMMUNITY_NAMES;
}

/** True when a label says nothing but which number the community got. */
function placeholderLabel(label: string, community: number): boolean {
  const trimmed = label.trim();
  return trimmed === ""
    || trimmed === String(community)
    || trimmed === `Community ${community}`;
}

/**
 * The graph with the hierarchy's community names applied: a community bubble
 * is named by its community, a symbol keeps its own name and gains the
 * community it belongs to. Labels an export already published — `labels.json`
 * survives the export — win over the hierarchy's, so a reader's vocabulary is
 * never replaced by a derived one.
 */
export function withCommunityNames(
  model: GraphViewModel,
  names: ReadonlyMap<number, string>
): GraphViewModel {
  if (names.size === 0) return model;
  let changed = false;
  const nodes = model.nodes.map((node) => {
    const name = names.get(node.community);
    if (name === undefined) return node;
    const label = node.kind === "community"
      && placeholderLabel(node.label, node.community)
      ? name
      : node.label;
    // A symbol keeps its own name and takes the community it belongs to; a
    // placeholder an export published is replaced the same way its labels are.
    const communityName = node.communityName === undefined
      || placeholderLabel(node.communityName, node.community)
      ? name
      : node.communityName;
    if (label === node.label && communityName === node.communityName) return node;
    changed = true;
    return { ...node, label, communityName };
  });
  const communities = model.communities.map((community) => {
    const name = names.get(community.id);
    if (name === undefined || !placeholderLabel(community.label, community.id)) {
      return community;
    }
    if (name === community.label) return community;
    changed = true;
    return { ...community, label: name };
  });
  return changed ? { ...model, nodes, communities } : model;
}

/** The community evidence a selected bubble shows in the inspector. */
export type CommunityFacts = {
  label: string;
  /** Symbols the community stands for. */
  symbols: number;
  /** Durable identity of the published group, when the node names one. */
  groupId?: string;
  communityId?: number;
  /** Level that published the group, and the rule that produced the level. */
  level?: number;
  merge?: HierarchyLevel["merge"];
  resolution?: number;
  /** Sub-groups the reader can descend into. */
  childGroups?: number;
  cohesion?: number;
  conductance?: number;
  boundaryKinds: ReadonlyArray<readonly [string, number]>;
  /** Relationships this community keeps with the groups beside it. */
  couplings: ReadonlyArray<{ label: string; edges: number }>;
  /** Every relationship the community keeps, before the list is bounded. */
  couplingCount: number;
  /** True when every coupling of the level projection is listed. */
  couplingsComplete: boolean;
};

type LocatedGroup = {
  levelIndex: number;
  level: HierarchyLevel;
  group: HierarchyGroup;
};

/**
 * The published group a bubble stands for: by the durable id the level
 * projections name their nodes with, then by the community the finest level
 * pairs with its group.
 */
function locateGroup(
  hierarchy: CommunityHierarchyView | undefined,
  node: GraphNode,
  communityKeyed: boolean
): LocatedGroup | undefined {
  if (!hierarchy) return undefined;
  for (const [levelIndex, level] of hierarchy.levels.entries()) {
    const group = level.groups.find((candidate) => candidate.id === node.id);
    if (group) return { levelIndex, level, group };
  }
  // A level projection numbers its nodes by group index, so only a projection
  // keyed by published community ids may fall back to the community pairing.
  if (!communityKeyed) return undefined;
  const finestIndex = hierarchy.levels.length - 1;
  const finest = hierarchy.levels[finestIndex];
  const group = finest?.groups.find((candidate) => candidate.community === node.community);
  return group && finest ? { levelIndex: finestIndex, level: finest, group } : undefined;
}

/** A community's couplings in the level projection that draws it. */
function couplings(
  located: LocatedGroup | undefined,
  model: GraphViewModel,
  node: GraphNode
): {
  edges: ReadonlyArray<{ label: string; edges: number }>;
  count: number;
  complete: boolean;
} {
  const levelModel = located?.level.model;
  const subject = located && levelModel
    ? levelModel.nodes.find((candidate) => candidate.id === located.group.id)
    : undefined;
  const graph = levelModel && subject ? levelModel : model;
  const subjectId = subject?.id ?? node.id;
  const byId = new Map(graph.nodes.map((candidate) => [candidate.id, candidate]));
  const counts = new Map<string, number>();
  for (const edge of graph.edges) {
    const other = edge.source === subjectId
      ? edge.target
      : edge.target === subjectId ? edge.source : undefined;
    if (other === undefined) continue;
    counts.set(other, (counts.get(other) ?? 0) + (edge.weight ?? 1));
  }
  const ordered = [...counts.entries()]
    .map(([id, edges]) => ({ label: byId.get(id)?.label ?? id, edges }))
    .sort((left, right) => right.edges - left.edges
      || left.label.localeCompare(right.label));
  return {
    edges: ordered.slice(0, COMMUNITY_COUPLING_LIMIT),
    count: ordered.reduce((total, entry) => total + entry.edges, 0),
    complete: ordered.length <= COMMUNITY_COUPLING_LIMIT
  };
}

/**
 * What the inspector shows for a community bubble. A hierarchy makes this
 * evidence-backed: the group's own members, sub-groups, identity, and the
 * metrics and couplings the published projection carries. A bubble from an
 * export without a hierarchy still reports the members it stands for and the
 * relationships the drawn graph holds.
 */
export function communityFacts(
  hierarchy: CommunityHierarchyView | undefined,
  model: GraphViewModel,
  node: GraphNode,
  options: { communityKeyed?: boolean | undefined } = {}
): CommunityFacts | undefined {
  const bubble = node.kind === "community" || model.stats.aggregated;
  if (!bubble) return undefined;
  const located = locateGroup(hierarchy, node, options.communityKeyed ?? true);
  const symbols = located?.group.memberCount ?? node.memberCount;
  if (located === undefined && symbols === undefined) return undefined;
  const listed = couplings(located, model, node);
  const boundaryKinds = Object.entries(located?.group.boundaryKinds ?? {})
    .filter(([, count]) => count > 0)
    .sort(([left], [right]) => left.localeCompare(right))
    .slice(0, COMMUNITY_BOUNDARY_LIMIT);
  return {
    label: located?.group.label ?? node.communityName ?? node.label,
    symbols: symbols ?? 0,
    ...(located ? { groupId: located.group.id } : {}),
    ...(located?.group.community === undefined
      ? {}
      : { communityId: located.group.community }),
    ...(located
      ? {
        level: located.level.level,
        merge: located.level.merge,
        ...(located.level.resolution === undefined
          ? {}
          : { resolution: located.level.resolution }),
        childGroups: located.group.childIndices.length,
        cohesion: located.group.cohesion,
        conductance: located.group.conductance
      }
      : {}),
    boundaryKinds,
    couplings: listed.edges,
    couplingCount: listed.count,
    couplingsComplete: listed.complete
  };
}
