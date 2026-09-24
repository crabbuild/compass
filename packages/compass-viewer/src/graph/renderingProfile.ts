import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";
import {
  communityOverviewLabelText,
  communityOverviewLabelledIds
} from "./communityOverview";

export const STATIC_LAYOUT_NODE_THRESHOLD = 1_000;
export const STATIC_LAYOUT_EDGE_THRESHOLD = 4_000;
export const AGGREGATED_EDGE_RENDER_LIMIT = 4_000;

export type GraphRenderingProfile = "interactive" | "static";
export type GraphLayoutStyle =
  | "automatic"
  | "hierarchical"
  | "circle"
  | "concentric"
  | "spiral"
  | "grid";

export function graphRenderingProfile(model: GraphViewModel): GraphRenderingProfile {
  return model.nodes.length >= STATIC_LAYOUT_NODE_THRESHOLD
    || model.edges.length >= STATIC_LAYOUT_EDGE_THRESHOLD
    ? "static"
    : "interactive";
}

/**
 * Whether the Automatic layout arranges itself when the view opens. The budget
 * is the interactive profile: a force simulation over those graphs settles
 * within the arranging screen, while a larger graph keeps its deterministic
 * seeded map and waits for an explicit "Layout" action instead of blocking the
 * first frame.
 */
export function autoLayoutApplies(model: GraphViewModel): boolean {
  return graphRenderingProfile(model) === "interactive";
}

export function seedStaticGraphPositions(
  nodes: readonly GraphNode[],
  aggregated = false
): ReadonlyMap<string, { x: number; y: number }> {
  if (aggregated) return seedCommunityOverviewPositions(nodes);

  const grouped = new Map<number, GraphNode[]>();
  for (const node of nodes) {
    const group = grouped.get(node.community) ?? [];
    group.push(node);
    grouped.set(node.community, group);
  }

  const groups = [...grouped.entries()]
    .sort(([left, leftMembers], [right, rightMembers]) =>
      rightMembers.length - leftMembers.length
      || left - right)
    .map(([community, members]) => ({
      community,
      members: members.sort((left, right) => left.id.localeCompare(right.id))
    }));
  const memberRadius = groups.map((group) => memberClusterRadius(group.members.length));
  // Big communities hold the middle, the long tail spreads outward: the flat
  // map reads as one structure instead of a field of equally important discs.
  const centers = spreadCentersByImportance(groups.map((group, index) => ({
    id: String(group.community),
    radius: memberRadius[index] ?? 0
  })));
  const goldenAngle = Math.PI * (3 - Math.sqrt(5));
  const positions = new Map<string, { x: number; y: number }>();

  for (const group of groups) {
    const center = centers.get(String(group.community)) ?? { x: 0, y: 0 };
    group.members.forEach((node, nodeIndex) => {
      if (nodeIndex === 0) {
        positions.set(node.id, { x: center.x, y: center.y });
        return;
      }
      const radius = 24 * Math.sqrt(nodeIndex);
      const angle = nodeIndex * goldenAngle;
      positions.set(node.id, {
        x: center.x + Math.cos(angle) * radius,
        y: center.y + Math.sin(angle) * radius
      });
    });
  }

  return positions;
}

export function seedGraphLayoutPositions(
  nodes: readonly GraphNode[],
  style: Exclude<GraphLayoutStyle, "automatic">,
  aggregated = false
): ReadonlyMap<string, { x: number; y: number }> {
  const communityOrdered = [...nodes].sort((left, right) =>
    left.community - right.community
    || (right.degree ?? 0) - (left.degree ?? 0)
    || left.id.localeCompare(right.id));
  if (style === "circle") {
    const radius = Math.max(160, communityOrdered.length * 44 / (2 * Math.PI));
    return new Map(communityOrdered.map((node, index) => {
      const angle = -Math.PI / 2
        + index * 2 * Math.PI / Math.max(1, communityOrdered.length);
      return [node.id, {
        x: Math.cos(angle) * radius,
        y: Math.sin(angle) * radius
      }];
    }));
  }
  if (style === "concentric") {
    const topologyOrdered = [...nodes].sort((left, right) =>
      (right.degree ?? 0) - (left.degree ?? 0)
      || (right.memberCount ?? 0) - (left.memberCount ?? 0)
      || left.community - right.community
      || left.id.localeCompare(right.id));
    return seedConcentricPositions(topologyOrdered);
  }
  if (style === "hierarchical") return seedHierarchicalPositions(communityOrdered);
  if (style === "spiral") return seedSpiralPositions(communityOrdered);

  return seedGroupedGridPositions(communityOrdered, aggregated);
}

function seedHierarchicalPositions(
  nodes: readonly GraphNode[]
): ReadonlyMap<string, { x: number; y: number }> {
  const levels = new Map<number, GraphNode[]>();
  for (const node of nodes) {
    const depth = node.depth ?? (node.root ? 0 : 1);
    const level = levels.get(depth) ?? [];
    level.push(node);
    levels.set(depth, level);
  }
  const ordered = [...levels.entries()].sort(([left], [right]) => left - right);
  const positions = new Map<string, { x: number; y: number }>();
  for (const [depth, level] of ordered) {
    level.sort((left, right) =>
      Number(right.root ?? false) - Number(left.root ?? false)
      || (right.degree ?? 0) - (left.degree ?? 0)
      || left.id.localeCompare(right.id));
    const spacing = 148;
    level.forEach((node, index) => positions.set(node.id, {
      x: (index - (level.length - 1) / 2) * spacing,
      y: depth * 168
    }));
  }
  return positions;
}

function seedConcentricPositions(
  ordered: readonly GraphNode[]
): ReadonlyMap<string, { x: number; y: number }> {
  const positions = new Map<string, { x: number; y: number }>();
  const center = ordered[0];
  if (!center) return positions;
  positions.set(center.id, { x: 0, y: 0 });
  let nodeIndex = 1;
  let ring = 1;
  while (nodeIndex < ordered.length) {
    const radius = ring * 72;
    const capacity = Math.max(6, Math.floor(2 * Math.PI * radius / 56));
    const count = Math.min(capacity, ordered.length - nodeIndex);
    for (let slot = 0; slot < count; slot += 1) {
      const node = ordered[nodeIndex + slot];
      if (!node) continue;
      const angle = -Math.PI / 2 + slot * 2 * Math.PI / count;
      positions.set(node.id, {
        x: Math.cos(angle) * radius,
        y: Math.sin(angle) * radius
      });
    }
    nodeIndex += count;
    ring += 1;
  }
  return positions;
}

function seedSpiralPositions(
  ordered: readonly GraphNode[]
): ReadonlyMap<string, { x: number; y: number }> {
  const angleStep = 0.52;
  const spacing = 36;
  return new Map(ordered.map((node, index) => {
    if (index === 0) return [node.id, { x: 0, y: 0 }];
    const radius = spacing * Math.sqrt(index);
    const angle = index * angleStep;
    return [node.id, {
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius
    }];
  }));
}

function seedGroupedGridPositions(
  ordered: readonly GraphNode[],
  aggregated: boolean
): ReadonlyMap<string, { x: number; y: number }> {
  const nodeSpacing = 56;
  const groupGutter = 112;
  const aggregatedGroupSize = 16;
  const grouped = new Map<number, GraphNode[]>();
  ordered.forEach((node, index) => {
    const key = aggregated
      ? Math.floor(index / aggregatedGroupSize)
      : node.community;
    const members = grouped.get(key) ?? [];
    members.push(node);
    grouped.set(key, members);
  });
  const groups = [...grouped.entries()]
    .sort(([left], [right]) => left - right)
    .map(([key, members]) => {
      const columns = Math.max(1, Math.ceil(Math.sqrt(members.length)));
      const rows = Math.max(1, Math.ceil(members.length / columns));
      return {
        key,
        members,
        columns,
        rows,
        width: (columns - 1) * nodeSpacing,
        height: (rows - 1) * nodeSpacing
      };
    });
  const outerColumns = Math.max(1, Math.ceil(Math.sqrt(groups.length)));
  const outerRows = Math.max(1, Math.ceil(groups.length / outerColumns));
  const columnWidths = Array.from({ length: outerColumns }, () => 0);
  const rowHeights = Array.from({ length: outerRows }, () => 0);
  groups.forEach((group, index) => {
    const column = index % outerColumns;
    const row = Math.floor(index / outerColumns);
    columnWidths[column] = Math.max(columnWidths[column] ?? 0, group.width);
    rowHeights[row] = Math.max(rowHeights[row] ?? 0, group.height);
  });
  const columnCenters = centeredSlotCenters(columnWidths, groupGutter);
  const rowCenters = centeredSlotCenters(rowHeights, groupGutter);
  const positions = new Map<string, { x: number; y: number }>();
  groups.forEach((group, groupIndex) => {
    const outerColumn = groupIndex % outerColumns;
    const outerRow = Math.floor(groupIndex / outerColumns);
    group.members.forEach((node, nodeIndex) => {
      const row = Math.floor(nodeIndex / group.columns);
      const nodesInRow = Math.min(
        group.columns,
        group.members.length - row * group.columns
      );
      const column = nodeIndex % group.columns;
      positions.set(node.id, {
        x: (columnCenters[outerColumn] ?? 0)
          + (column - (nodesInRow - 1) / 2) * nodeSpacing,
        y: (rowCenters[outerRow] ?? 0)
          + (row - (group.rows - 1) / 2) * nodeSpacing
      });
    });
  });
  return positions;
}

function centeredSlotCenters(sizes: readonly number[], gutter: number): number[] {
  const total = sizes.reduce((sum, size) => sum + size, 0)
    + Math.max(0, sizes.length - 1) * gutter;
  let cursor = -total / 2;
  return sizes.map((size) => {
    const center = cursor + size / 2;
    cursor += size + gutter;
    return center;
  });
}

export function visibleGraphEdges(model: GraphViewModel): readonly GraphEdge[] {
  if (!model.stats.aggregated || model.edges.length <= AGGREGATED_EDGE_RENDER_LIMIT) {
    return model.edges;
  }
  const ordered = [...model.edges].sort((left, right) =>
    (right.weight ?? relationWeight(right.relation))
      - (left.weight ?? relationWeight(left.relation))
    || left.source.localeCompare(right.source)
    || left.target.localeCompare(right.target)
    || left.id.localeCompare(right.id));
  const parent = new Map(model.nodes.map((node) => [node.id, node.id]));
  const find = (id: string): string => {
    let root = parent.get(id) ?? id;
    while (parent.get(root) !== undefined && parent.get(root) !== root) {
      root = parent.get(root)!;
    }
    let current = id;
    while (parent.get(current) !== undefined && parent.get(current) !== root) {
      const next = parent.get(current)!;
      parent.set(current, root);
      current = next;
    }
    return root;
  };
  const selected: GraphEdge[] = [];
  const selectedIds = new Set<string>();
  for (const edge of ordered) {
    const source = find(edge.source);
    const target = find(edge.target);
    if (source === target) continue;
    parent.set(target, source);
    selected.push(edge);
    selectedIds.add(edge.id);
    if (selected.length === AGGREGATED_EDGE_RENDER_LIMIT) return selected;
  }
  for (const edge of ordered) {
    if (selectedIds.has(edge.id)) continue;
    selected.push(edge);
    if (selected.length === AGGREGATED_EDGE_RENDER_LIMIT) break;
  }
  return selected;
}

function relationWeight(relation: string): number {
  const parsed = Number.parseInt(relation, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 1;
}

const CLUSTER_GAP = 14;
/**
 * Shape of a packed overview. The graph stage is roughly as tall as it is wide
 * once the navigation rail and inspector are on screen, so packing for a wide
 * canvas would leave the fitted map with empty margins above and below.
 */
const CLUSTER_TARGET_ASPECT = 1.35;
const MEMBER_SPACING = 24;
/** Largest horizontal stretch applied to fit a radial map to a wide canvas. */
const CLUSTER_MAXIMUM_STRETCH = 1.6;
const COMMUNITY_BUBBLE_PADDING = 10;
const COMMUNITY_LABEL_LINE_HEIGHT = 28;
const COMMUNITY_LABEL_MAXIMUM_HALF_WIDTH = 104;
const COMMUNITY_LABEL_CHARACTER_WIDTH = 3.9;

/**
 * Radius of the disc that holds one community's members, so communities can be
 * packed as discs instead of being dropped into fixed grid cells.
 */
function memberClusterRadius(memberCount: number): number {
  return MEMBER_SPACING * Math.sqrt(Math.max(1, memberCount - 1)) + 18;
}

type ClusterFootprint = {
  id: string;
  halfWidth: number;
  halfHeight: number;
};

/**
 * Space one community bubble claims on the overview map: its rendered bubble,
 * and for labelled bubbles the room its label needs below and beside it.
 */
function communityBubbleFootprint(
  node: GraphNode,
  labelled: boolean
): ClusterFootprint {
  const radius = node.size ?? 12;
  if (!labelled) {
    return {
      id: node.id,
      halfWidth: radius + COMMUNITY_BUBBLE_PADDING * 0.6,
      halfHeight: radius + COMMUNITY_BUBBLE_PADDING * 0.6
    };
  }
  const characters = communityOverviewLabelText(node.label).length;
  return {
    id: node.id,
    halfWidth: Math.max(
      radius,
      Math.min(
        COMMUNITY_LABEL_MAXIMUM_HALF_WIDTH,
        characters * COMMUNITY_LABEL_CHARACTER_WIDTH
      )
    ) + COMMUNITY_BUBBLE_PADDING,
    halfHeight: radius + COMMUNITY_LABEL_LINE_HEIGHT
  };
}

/**
 * Deterministically pack footprints row by row, sizing the rows so the packed
 * block matches the canvas aspect. A shelf packing keeps the map compact enough
 * that a fitted view still renders its labels at a readable size, and it never
 * depends on input order.
 */
export function seedCommunityOverviewPositions(
  nodes: readonly GraphNode[],
  importance?: ReadonlyMap<string, number> | undefined
): ReadonlyMap<string, { x: number; y: number }> {
  const ordered = communityOverviewOrder(nodes, importance);
  const labelled = communityOverviewLabelledIds(ordered, importance);
  const footprints = ordered.map((node) =>
    communityBubbleFootprint(node, labelled.has(node.id)));
  const seeded = communitySeedPositions(ordered, footprints);
  return centerClusterPositions(
    relaxCommunityOverviewPositions(ordered, seeded, importance),
    footprints
  );
}

/**
 * The pre-packing seed positions.
 *
 * Rank sets the radius, so importance still anchors the centre, while an
 * identity-derived bearing sets the direction: an untouched group keeps its
 * bearing when a neighbour grows, appears, or disappears, and the packing pass
 * only has to resolve overlaps from a stable starting point.
 */
export function communitySeedPositions(
  ordered: readonly GraphNode[],
  footprints: readonly ClusterFootprint[]
): ReadonlyMap<string, { x: number; y: number }> {
  // Golden-angle spiral: the most important community anchors the centre and
  // each rank sits a little further out, so the tail lands outside the core
  // instead of being packed into the same field. The *bearing* comes from the
  // node's identity rather than its rank, so a group that keeps its evidence
  // keeps its direction when another group grows, shrinks, or appears.
  const meanArea = footprints.reduce(
    (sum, footprint) => sum + 4 * footprint.halfWidth * footprint.halfHeight,
    0
  ) / Math.max(1, footprints.length);
  const spacing = Math.sqrt(meanArea / Math.PI);
  const seeded = new Map<string, { x: number; y: number }>(ordered.map((node, index) => [
    node.id,
    index === 0
      ? { x: 0, y: 0 }
      : {
        x: Math.cos(stableBearing(node.id)) * spacing * Math.sqrt(index),
        y: Math.sin(stableBearing(node.id)) * spacing * Math.sqrt(index)
      }
  ]));
  return seeded;
}

/**
 * A deterministic bearing in `[0, 2π)` derived from an identity.
 *
 * Position stability needs this: seeding by arrival order or rank means an
 * unrelated group changing size rotates every bubble around it.
 */
export function stableBearing(id: string): number {
  let hash = 2_166_136_261;
  for (let index = 0; index < id.length; index += 1) {
    hash ^= id.charCodeAt(index);
    hash = Math.imul(hash, 16_777_619);
  }
  return ((hash >>> 0) % 4096) / 4096 * Math.PI * 2;
}

/**
 * Origin the packed map and widen it toward the canvas aspect ratio. Stretching
 * only grows horizontal distances, so a non-overlapping layout stays
 * non-overlapping while the fitted view stops wasting the side margins.
 */
function centerClusterPositions(
  positions: ReadonlyMap<string, { x: number; y: number }>,
  footprints: readonly ClusterFootprint[]
): ReadonlyMap<string, { x: number; y: number }> {
  const footprintOf = new Map(footprints.map((footprint) => [footprint.id, footprint]));
  const entries = [...positions.entries()].sort(([left], [right]) =>
    left.localeCompare(right));
  if (entries.length === 0) return positions;
  let left = Number.POSITIVE_INFINITY;
  let right = Number.NEGATIVE_INFINITY;
  let top = Number.POSITIVE_INFINITY;
  let bottom = Number.NEGATIVE_INFINITY;
  for (const [id, { x, y }] of entries) {
    const footprint = footprintOf.get(id);
    left = Math.min(left, x - (footprint?.halfWidth ?? 0));
    right = Math.max(right, x + (footprint?.halfWidth ?? 0));
    top = Math.min(top, y - (footprint?.halfHeight ?? 0));
    bottom = Math.max(bottom, y + (footprint?.halfHeight ?? 0));
  }
  const width = Math.max(1, right - left);
  const height = Math.max(1, bottom - top);
  const stretch = Math.min(
    CLUSTER_MAXIMUM_STRETCH,
    Math.max(1, (CLUSTER_TARGET_ASPECT * height) / width)
  );
  const centerX = (left + right) / 2;
  const centerY = (top + bottom) / 2;
  return new Map(entries.map(([id, { x, y }]) => [
    id,
    {
      x: (x - centerX) * stretch,
      y: y - centerY
    }
  ]));
}

/**
 * Importance order used by every community layout: the reader's ranking decides
 * which communities anchor the centre and which are scattered at the rim.
 */
export function communityOverviewOrder(
  nodes: readonly GraphNode[],
  importance?: ReadonlyMap<string, number> | undefined
): GraphNode[] {
  return [...nodes].sort((left, right) =>
    (importance?.get(right.id) ?? 0) - (importance?.get(left.id) ?? 0)
    || (right.memberCount ?? 0) - (left.memberCount ?? 0)
    || (right.degree ?? 0) - (left.degree ?? 0)
    || left.id.localeCompare(right.id));
}

/**
 * Re-centre a settled community map on its importance ranking: each node keeps
 * the direction the simulation gave it (which encodes its couplings) and moves
 * toward the radius its rank earns. Important communities end up in a compact
 * core, the long tail scatters around them.
 */
export function centerCommunityOverviewPositions(
  nodes: readonly GraphNode[],
  positions: ReadonlyMap<string, { x: number; y: number }>,
  importance?: ReadonlyMap<string, number> | undefined,
  blend = 0.7
): ReadonlyMap<string, { x: number; y: number }> {
  const ordered = communityOverviewOrder(nodes, importance)
    .filter((node) => positions.has(node.id));
  if (ordered.length < 2) return new Map(positions);
  let centerX = 0;
  let centerY = 0;
  for (const node of ordered) {
    const position = positions.get(node.id)!;
    centerX += position.x;
    centerY += position.y;
  }
  centerX /= ordered.length;
  centerY /= ordered.length;
  const extent = ordered.reduce((largest, node) => {
    const position = positions.get(node.id)!;
    return Math.max(largest, Math.hypot(position.x - centerX, position.y - centerY));
  }, 0) || 1;
  const goldenAngle = Math.PI * (3 - Math.sqrt(5));
  const ratio = Math.min(1, Math.max(0, blend));
  return new Map(ordered.map((node, index) => {
    const position = positions.get(node.id)!;
    const dx = position.x - centerX;
    const dy = position.y - centerY;
    const distance = Math.hypot(dx, dy);
    // Rank 0 sits at the centre; the last rank takes the outer edge.
    const targetRadius = extent * Math.sqrt(index / (ordered.length - 1));
    const angle = distance > 1e-6 ? Math.atan2(dy, dx) : index * goldenAngle;
    const radius = distance * (1 - ratio) + targetRadius * ratio;
    return [node.id, {
      x: centerX + Math.cos(angle) * radius,
      y: centerY + Math.sin(angle) * radius
    }];
  }));
}

/**
 * Centre-first disc packing for flat community maps: the largest community
 * takes the middle and every next disc settles as close to it as its own radius
 * allows, spiralling outward. Radius-aware steps keep the big discs central
 * instead of letting collisions shove them to the rim, and the walk is
 * deterministic for identical input.
 */
export function spreadCentersByImportance(
  discs: ReadonlyArray<{ id: string; radius: number }>
): ReadonlyMap<string, { x: number; y: number }> {
  const positions = new Map<string, { x: number; y: number }>();
  if (discs.length === 0) return positions;
  const largestRadius = discs.reduce(
    (largest, disc) => Math.max(largest, disc.radius),
    0
  );
  const meanRadius = discs.reduce((sum, disc) => sum + disc.radius, 0) / discs.length;
  const cellSize = Math.max(48, largestRadius * 2 + CLUSTER_GAP);
  const goldenAngle = Math.PI * (3 - Math.sqrt(5));
  const cells = new Map<string, Array<{ x: number; y: number; radius: number }>>();
  const cellKey = (x: number, y: number) =>
    `${Math.floor(x / cellSize)}:${Math.floor(y / cellSize)}`;
  const fits = (x: number, y: number, radius: number): boolean => {
    const column = Math.floor(x / cellSize);
    const row = Math.floor(y / cellSize);
    for (let offsetX = -1; offsetX <= 1; offsetX += 1) {
      for (let offsetY = -1; offsetY <= 1; offsetY += 1) {
        for (const placed of cells.get(`${column + offsetX}:${row + offsetY}`) ?? []) {
          if (Math.hypot(placed.x - x, placed.y - y) < placed.radius + radius + CLUSTER_GAP) {
            return false;
          }
        }
      }
    }
    return true;
  };
  const place = (disc: { id: string; radius: number }, x: number, y: number) => {
    positions.set(disc.id, { x, y });
    const key = cellKey(x, y);
    const bucket = cells.get(key) ?? [];
    bucket.push({ x, y, radius: disc.radius });
    cells.set(key, bucket);
  };
  const idealSpacing = 2 * meanRadius + CLUSTER_GAP;
  discs.forEach((disc, index) => {
    if (index === 0) {
      place(disc, 0, 0);
      return;
    }
    const idealRadius = idealSpacing * Math.sqrt(index);
    const idealAngle = index * goldenAngle;
    const ideal = {
      x: Math.cos(idealAngle) * idealRadius,
      y: Math.sin(idealAngle) * idealRadius
    };
    if (fits(ideal.x, ideal.y, disc.radius)) {
      place(disc, ideal.x, ideal.y);
      return;
    }
    const stepLength = Math.max(10, disc.radius * 0.4 + CLUSTER_GAP * 0.5);
    for (let step = 1; step <= 480; step += 1) {
      const angle = step * goldenAngle;
      const distance = stepLength * Math.sqrt(step);
      const x = ideal.x + Math.cos(angle) * distance;
      const y = ideal.y + Math.sin(angle) * distance;
      if (fits(x, y, disc.radius)) {
        place(disc, x, y);
        return;
      }
    }
    place(disc, ideal.x, ideal.y);
  });
  return centerClusterPositions(
    positions,
    discs.map((disc) => ({
      id: disc.id,
      halfWidth: disc.radius,
      halfHeight: disc.radius
    }))
  );
}

/**
 * Push placed footprints apart until none overlap. Layout passes own the
 * structure (couplings for an arrangement, rank for a radial map); this pass
 * only removes the collisions they leave behind, so labels stay legible and the
 * result is deterministic for identical input. Movement is the minimum needed:
 * each pair separates along its shallower axis, half a penetration each.
 */
export function relaxFootprintPositions(
  footprints: readonly ClusterFootprint[],
  positions: ReadonlyMap<string, { x: number; y: number }>,
  iterations = 48
): ReadonlyMap<string, { x: number; y: number }> {
  const entries = [...footprints]
    .sort((left, right) => left.id.localeCompare(right.id))
    .flatMap((footprint) => {
      const start = positions.get(footprint.id);
      if (!start) return [];
      return [{ ...footprint, x: start.x, y: start.y }];
    });
  if (entries.length < 2) {
    return new Map(entries.map((entry) => [entry.id, { x: entry.x, y: entry.y }]));
  }
  const largestHalfExtent = entries.reduce(
    (largest, entry) => Math.max(largest, entry.halfWidth, entry.halfHeight),
    1
  );
  const cellSize = largestHalfExtent * 2 + CLUSTER_GAP;
  const keysFor = (entry: { x: number; y: number; halfWidth: number; halfHeight: number }) => {
    const keys: string[] = [];
    const left = Math.floor((entry.x - entry.halfWidth - CLUSTER_GAP) / cellSize);
    const right = Math.floor((entry.x + entry.halfWidth + CLUSTER_GAP) / cellSize);
    const top = Math.floor((entry.y - entry.halfHeight - CLUSTER_GAP) / cellSize);
    const bottom = Math.floor((entry.y + entry.halfHeight + CLUSTER_GAP) / cellSize);
    for (let column = left; column <= right; column += 1) {
      for (let row = top; row <= bottom; row += 1) {
        keys.push(`${column}:${row}`);
      }
    }
    return keys;
  };
  const passes = Math.max(1, Math.trunc(iterations));
  for (let pass = 0; pass < passes; pass += 1) {
    const cells = new Map<string, typeof entries>();
    for (const entry of entries) {
      for (const key of keysFor(entry)) {
        const bucket = cells.get(key) ?? [];
        bucket.push(entry);
        cells.set(key, bucket);
      }
    }
    let moved = 0;
    const visited = new Set<string>();
    for (const entry of entries) {
      for (const key of keysFor(entry)) {
        for (const other of cells.get(key) ?? []) {
          if (other.id <= entry.id) continue;
          const pair = `${entry.id}\u0000${other.id}`;
          if (visited.has(pair)) continue;
          visited.add(pair);
          const dx = other.x - entry.x;
          const dy = other.y - entry.y;
          const overlapX = entry.halfWidth + other.halfWidth + CLUSTER_GAP - Math.abs(dx);
          const overlapY = entry.halfHeight + other.halfHeight + CLUSTER_GAP - Math.abs(dy);
          if (overlapX <= 0 || overlapY <= 0) continue;
          const share = 0.5;
          if (overlapX < overlapY) {
            const direction = dx === 0 ? (entry.id < other.id ? -1 : 1) : Math.sign(dx);
            entry.x -= direction * overlapX * share;
            other.x += direction * overlapX * share;
          } else {
            const direction = dy === 0 ? (entry.id < other.id ? -1 : 1) : Math.sign(dy);
            entry.y -= direction * overlapY * share;
            other.y += direction * overlapY * share;
          }
          moved += 1;
        }
      }
    }
    if (moved === 0) break;
  }
  return new Map(entries.map((entry) => [entry.id, { x: entry.x, y: entry.y }]));
}

/**
 * Community bubbles with their label room, relaxed to remove the collisions an
 * arrangement leaves behind.
 */
export function relaxCommunityOverviewPositions(
  nodes: readonly GraphNode[],
  positions: ReadonlyMap<string, { x: number; y: number }>,
  importance?: ReadonlyMap<string, number> | undefined,
  iterations = 48
): ReadonlyMap<string, { x: number; y: number }> {
  const labelled = communityOverviewLabelledIds(nodes, importance);
  return relaxFootprintPositions(
    nodes.map((node) => communityBubbleFootprint(node, labelled.has(node.id))),
    positions,
    iterations
  );
}
