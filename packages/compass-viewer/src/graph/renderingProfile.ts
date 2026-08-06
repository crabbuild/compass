import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";

export const STATIC_LAYOUT_NODE_THRESHOLD = 1_000;
export const STATIC_LAYOUT_EDGE_THRESHOLD = 4_000;
export const AGGREGATED_EDGE_RENDER_LIMIT = 4_000;

export type GraphRenderingProfile = "interactive" | "static";
export type GraphLayoutStyle =
  | "automatic"
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

export function seedStaticGraphPositions(
  nodes: readonly GraphNode[],
  aggregated = false
): ReadonlyMap<string, { x: number; y: number }> {
  if (aggregated) return seedAggregatedGraphPositions(nodes);

  const grouped = new Map<number, GraphNode[]>();
  for (const node of nodes) {
    const group = grouped.get(node.community) ?? [];
    group.push(node);
    grouped.set(node.community, group);
  }

  const groups = [...grouped.entries()]
    .sort(([left], [right]) => left - right)
    .map(([community, members]) => ({
      community,
      members: members.sort((left, right) => left.id.localeCompare(right.id))
    }));
  const largestGroup = groups.reduce(
    (largest, group) => Math.max(largest, group.members.length),
    1
  );
  const columns = Math.max(1, Math.ceil(Math.sqrt(groups.length)));
  const rows = Math.max(1, Math.ceil(groups.length / columns));
  const cellSize = Math.max(280, Math.ceil(Math.sqrt(largestGroup)) * 48);
  const goldenAngle = Math.PI * (3 - Math.sqrt(5));
  const positions = new Map<string, { x: number; y: number }>();

  groups.forEach((group, groupIndex) => {
    const column = groupIndex % columns;
    const row = Math.floor(groupIndex / columns);
    const centerX = (column - (columns - 1) / 2) * cellSize;
    const centerY = (row - (rows - 1) / 2) * cellSize;
    group.members.forEach((node, nodeIndex) => {
      if (nodeIndex === 0) {
        positions.set(node.id, { x: centerX, y: centerY });
        return;
      }
      const radius = 24 * Math.sqrt(nodeIndex);
      const angle = nodeIndex * goldenAngle;
      positions.set(node.id, {
        x: centerX + Math.cos(angle) * radius,
        y: centerY + Math.sin(angle) * radius
      });
    });
  });

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
  if (style === "spiral") return seedSpiralPositions(communityOrdered);

  return seedGroupedGridPositions(communityOrdered, aggregated);
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

function seedAggregatedGraphPositions(
  nodes: readonly GraphNode[]
): ReadonlyMap<string, { x: number; y: number }> {
  const ordered = [...nodes].sort((left, right) =>
    (right.degree ?? 0) - (left.degree ?? 0)
    || (right.memberCount ?? 0) - (left.memberCount ?? 0)
    || left.id.localeCompare(right.id));
  const goldenAngle = Math.PI * (3 - Math.sqrt(5));
  const spacing = 34;
  return new Map(ordered.map((node, index) => {
    if (index === 0) return [node.id, { x: 0, y: 0 }];
    const radius = spacing * Math.sqrt(index);
    const angle = index * goldenAngle;
    return [node.id, {
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius
    }];
  }));
}
