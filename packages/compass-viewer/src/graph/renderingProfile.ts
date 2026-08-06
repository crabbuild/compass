import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";

export const STATIC_LAYOUT_NODE_THRESHOLD = 1_000;
export const STATIC_LAYOUT_EDGE_THRESHOLD = 4_000;
export const AGGREGATED_EDGE_RENDER_LIMIT = 4_000;

export type GraphRenderingProfile = "interactive" | "static";

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
