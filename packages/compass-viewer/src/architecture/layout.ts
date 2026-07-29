import type {
  ArchitectureRouteSummary,
  ArchitectureSectionSummary
} from "../contracts/architecture";

export type ArchitectureLayoutNode = ArchitectureSectionSummary & {
  x: number;
  y: number;
  width: number;
  height: number;
  column: number;
};

export type ArchitectureLayoutRoute = ArchitectureRouteSummary & {
  path: string;
  width: number;
};

export type ArchitectureLayout = {
  width: number;
  height: number;
  nodes: ArchitectureLayoutNode[];
  routes: ArchitectureLayoutRoute[];
};

export function layoutArchitecture(
  sections: readonly ArchitectureSectionSummary[],
  routes: readonly ArchitectureRouteSummary[],
  viewport: { width: number; height: number } = { width: 1080, height: 640 }
): ArchitectureLayout {
  const visible = sections.filter((section) =>
    section.nodeCount > 0 || section.incomingCalls > 0 || section.outgoingCalls > 0
  );
  if (visible.length === 0) {
    return { width: viewport.width, height: viewport.height, nodes: [], routes: [] };
  }
  const columns = Math.min(4, Math.max(1, Math.ceil(Math.sqrt(visible.length))));
  const ordered = [...visible].sort((left, right) => {
    const leftBias = left.incomingCalls - left.outgoingCalls;
    const rightBias = right.incomingCalls - right.outgoingCalls;
    return leftBias - rightBias
      || right.nodeCount - left.nodeCount
      || left.name.localeCompare(right.name);
  });
  const columnRows = Array.from({ length: columns }, () => [] as typeof ordered);
  ordered.forEach((section, index) => {
    const column = Math.min(columns - 1, Math.floor(index * columns / ordered.length));
    columnRows[column]!.push(section);
  });
  const horizontalGap = viewport.width / columns;
  const nodes = columnRows.flatMap((rows, column) => {
    const verticalGap = viewport.height / Math.max(1, rows.length);
    return rows.map((section, row) => {
      const height = 54 + Math.min(28, Math.log10(section.nodeCount + 1) * 8);
      return {
        ...section,
        x: column * horizontalGap + horizontalGap / 2,
        y: row * verticalGap + verticalGap / 2,
        width: Math.min(180, horizontalGap - 34),
        height,
        column
      };
    });
  });
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const layoutRoutes = routes.flatMap((route) => {
    const source = byId.get(route.sourceSection);
    const target = byId.get(route.targetSection);
    if (!source || !target) return [];
    const forward = target.x > source.x;
    const startX = source.x + (forward ? source.width / 2 : -source.width / 2);
    const endX = target.x + (forward ? -target.width / 2 : target.width / 2);
    const bend = Math.max(52, Math.abs(endX - startX) * 0.46);
    const path = forward
      ? `M ${startX} ${source.y} C ${startX + bend} ${source.y}, ${endX - bend} ${target.y}, ${endX} ${target.y}`
      : `M ${startX} ${source.y} C ${startX - bend} ${source.y - 72}, ${endX + bend} ${target.y - 72}, ${endX} ${target.y}`;
    return [{
      ...route,
      path,
      width: 1.3 + Math.min(7, Math.log10(route.calls + 1) * 2.2)
    }];
  });
  return { width: viewport.width, height: viewport.height, nodes, routes: layoutRoutes };
}
