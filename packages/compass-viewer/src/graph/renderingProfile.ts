import type { GraphNode, GraphViewModel } from "../contracts/graph";

export const STATIC_LAYOUT_NODE_THRESHOLD = 1_000;
export const STATIC_LAYOUT_EDGE_THRESHOLD = 4_000;

export type GraphRenderingProfile = "interactive" | "static";

export function graphRenderingProfile(model: GraphViewModel): GraphRenderingProfile {
  return model.nodes.length >= STATIC_LAYOUT_NODE_THRESHOLD
    || model.edges.length >= STATIC_LAYOUT_EDGE_THRESHOLD
    ? "static"
    : "interactive";
}

export function seedStaticGraphPositions(
  nodes: readonly GraphNode[]
): ReadonlyMap<string, { x: number; y: number }> {
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
