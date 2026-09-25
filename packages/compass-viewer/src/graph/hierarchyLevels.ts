import type {
  CommunityHierarchyView,
  HierarchyGroup,
  HierarchyLevel
} from "../contracts/hierarchy";
import type { GraphViewModel } from "../contracts/graph";

/**
 * A scope a reader can open: a published level, or the symbol canvas. The
 * toggle reads `Repository | <level labels> | Symbols`, bounded by the
 * artifact's levels.
 */
export type HierarchyScope =
  | { kind: "level"; level: number }
  | { kind: "symbols" };

export const ROOT_SCOPE: HierarchyScope = { kind: "level", level: 0 };

/** The model a scope draws, when the export embedded one for that level. */
export function scopeModel(
  hierarchy: CommunityHierarchyView,
  scope: HierarchyScope
): GraphViewModel | undefined {
  if (scope.kind === "symbols") {
    return undefined;
  }
  return hierarchy.levels.find((level) => level.level === scope.level)?.model;
}

/** The level a scope names, if the artifact publishes it. */
export function scopeLevel(
  hierarchy: CommunityHierarchyView,
  scope: HierarchyScope
): HierarchyLevel | undefined {
  if (scope.kind === "symbols") {
    return undefined;
  }
  return hierarchy.levels.find((level) => level.level === scope.level);
}

/** The group a scope narrowed to, for the breadcrumb and the inspector. */
export function groupAt(
  hierarchy: CommunityHierarchyView,
  scope: HierarchyScope,
  groupIndex: number | undefined
): HierarchyGroup | undefined {
  if (groupIndex === undefined) {
    return undefined;
  }
  return scopeLevel(hierarchy, scope)?.groups.find((group) => group.index === groupIndex);
}

/**
 * The children of one group inside the level below it, filtered from that
 * level's own projection: only the nodes and edges the group owns, so
 * descending reads as narrowing the canvas rather than redrawing it.
 */
export function descendModel(
  hierarchy: CommunityHierarchyView,
  level: number,
  groupIndex: number
): GraphViewModel | undefined {
  const parent = hierarchy.levels.find((entry) => entry.level === level);
  const child = hierarchy.levels.find((entry) => entry.level === level + 1);
  const group = parent?.groups.find((entry) => entry.index === groupIndex);
  if (!group || !child?.model) {
    return undefined;
  }
  // A level's projection names each node after the group it stands for, so the
  // children of a group are exactly the groups it indexes — by id, which
  // survives a rebuild that renumbers the level.
  const members = new Set(
    group.childIndices
      .map((index) => child.groups[index]?.id)
      .filter((id): id is string => typeof id === "string")
  );
  if (members.size === 0) {
    return undefined;
  }
  const nodes = child.model.nodes.filter((node) => members.has(node.id));
  const nodeIds = new Set(nodes.map((node) => node.id));
  const edges = child.model.edges.filter(
    (edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target)
  );
  // The panel, legend, and counts read `communities`, so a narrowed scope has
  // to publish the groups it actually draws instead of the whole level.
  const communityIds = new Set(nodes.map((node) => node.community));
  return {
    ...child.model,
    stats: {
      ...child.model.stats,
      nodes: nodes.length,
      edges: edges.length,
      communities: communityIds.size
    },
    nodes,
    edges,
    communities: child.model.communities.filter(
      (community) => communityIds.has(community.id)
    )
  };
}

/**
 * Whether the export published a drawable projection for every level, which is
 * what level navigation needs. A repository larger than the export budget
 * publishes some levels as groups only.
 */
export function hierarchyNavigable(hierarchy: CommunityHierarchyView): boolean {
  return hierarchy.levels.every((level) => level.model !== undefined);
}

/**
 * The level a reader opens on: the coarsest level that decomposes the
 * repository into more than one group and that the export could draw.
 *
 * A level holding one group is the whole repository drawn as a single node — a
 * tiny repository whose partition has one community, or an older artifact whose
 * coarsening collapsed every named group into one bucket. Opening on that node
 * shows a reader nothing, so the page falls back to the coarsest level that
 * actually splits the graph; only a hierarchy with no such level leaves the
 * reader on the symbol canvas. The level toggle still offers every published
 * level, so an explicit request for a single-group level is honoured.
 */
export function hierarchyOpeningLevel(
  hierarchy: CommunityHierarchyView
): number | undefined {
  const level = hierarchy.levels.find(
    (candidate) => candidate.groups.length > 1 && candidate.model !== undefined
  );
  return level?.level;
}

/**
 * The labels a breadcrumb shows for a scope: the repository, then the group the
 * reader descended through on each level above the active one.
 */
export function hierarchyTrail(
  hierarchy: CommunityHierarchyView,
  scope: HierarchyScope,
  trail: ReadonlyArray<{ level: number; groupIndex: number }>
): string[] {
  if (scope.kind === "symbols") {
    return ["Symbols"];
  }
  const labels = ["Repository"];
  for (const entry of trail) {
    const level = hierarchy.levels.find((candidate) => candidate.level === entry.level);
    const group = level?.groups.find((candidate) => candidate.index === entry.groupIndex);
    if (group) {
      labels.push(group.label);
    }
  }
  return labels;
}
