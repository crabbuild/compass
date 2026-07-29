import type {
  ArchitectureRouteSummary,
  ArchitectureSectionSummary
} from "../contracts/architecture";

export type ArchitecturePosition = {
  x: number;
  y: number;
};

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
  direction: "forward" | "backward" | "lateral";
};

export type ArchitectureLayoutLane = {
  index: number;
  x: number;
  width: number;
  label: string;
};

export type ArchitectureLayout = {
  width: number;
  height: number;
  nodes: ArchitectureLayoutNode[];
  routes: ArchitectureLayoutRoute[];
  lanes: ArchitectureLayoutLane[];
};

const NODE_WIDTH = 190;
const NODE_HEIGHT = 66;
const TOP_GUTTER = 102;
const BOTTOM_GUTTER = 78;
const SIDE_GUTTER = 72;

export function layoutArchitecture(
  sections: readonly ArchitectureSectionSummary[],
  routes: readonly ArchitectureRouteSummary[],
  viewport: { width: number; height: number } = { width: 1280, height: 760 },
  positions: Readonly<Record<string, ArchitecturePosition>> = {}
): ArchitectureLayout {
  const visible = sections.filter((section) =>
    section.nodeCount > 0 || section.incomingCalls > 0 || section.outgoingCalls > 0
  );
  if (visible.length === 0) {
    return {
      width: viewport.width,
      height: viewport.height,
      nodes: [],
      routes: [],
      lanes: []
    };
  }

  const columnCount = visible.length === 1
    ? 1
    : Math.min(5, Math.max(2, Math.round(Math.sqrt(visible.length))));
  const laneWidth = (viewport.width - SIDE_GUTTER * 2) / columnCount;
  const sectionIds = new Set(visible.map((section) => section.id));
  const visibleRoutes = routes.filter((route) =>
    sectionIds.has(route.sourceSection) && sectionIds.has(route.targetSection)
  );
  const columns = assignColumns(visible, columnCount);
  orderColumns(columns, visibleRoutes);

  const lanes = Array.from({ length: columnCount }, (_, index) => ({
    index,
    x: SIDE_GUTTER + laneWidth * index,
    width: laneWidth,
    label: laneLabel(index, columnCount)
  }));
  const availableHeight = viewport.height - TOP_GUTTER - BOTTOM_GUTTER;
  const nodes = columns.flatMap((column, columnIndex) => {
    const step = availableHeight / Math.max(1, column.length);
    return column.map((section, row) => {
      const automatic = {
        x: SIDE_GUTTER + laneWidth * columnIndex + laneWidth / 2,
        y: TOP_GUTTER + step * row + step / 2
      };
      const position = positions[section.id] ?? automatic;
      return {
        ...section,
        x: clamp(position.x, SIDE_GUTTER + NODE_WIDTH / 2, viewport.width - SIDE_GUTTER - NODE_WIDTH / 2),
        y: clamp(position.y, TOP_GUTTER, viewport.height - BOTTOM_GUTTER),
        width: Math.min(NODE_WIDTH, laneWidth - 30),
        height: NODE_HEIGHT,
        column: nearestColumn(position.x, lanes)
      };
    });
  });

  const byId = new Map(nodes.map((node) => [node.id, node]));
  const routeBands = new Map<string, number>();
  const layoutRoutes = visibleRoutes.flatMap((route) => {
    const source = byId.get(route.sourceSection);
    const target = byId.get(route.targetSection);
    if (!source || !target) return [];
    const direction = routeDirection(source, target);
    const bandKey = `${Math.min(source.column, target.column)}:${Math.max(source.column, target.column)}:${direction}`;
    const band = routeBands.get(bandKey) ?? 0;
    routeBands.set(bandKey, band + 1);
    return [{
      ...route,
      direction,
      path: routePath(source, target, direction, band, viewport),
      width: 1 + Math.min(4.4, Math.log10(route.calls + 1) * 1.35)
    }];
  });

  return {
    width: viewport.width,
    height: viewport.height,
    nodes,
    routes: layoutRoutes,
    lanes
  };
}

function assignColumns(
  sections: readonly ArchitectureSectionSummary[],
  columnCount: number
): ArchitectureSectionSummary[][] {
  const ordered = [...sections].sort((left, right) => {
    const leftFlow = flowBias(left);
    const rightFlow = flowBias(right);
    return rightFlow - leftFlow
      || (right.incomingCalls + right.outgoingCalls)
        - (left.incomingCalls + left.outgoingCalls)
      || left.name.localeCompare(right.name);
  });
  const columns = Array.from({ length: columnCount }, () => [] as ArchitectureSectionSummary[]);
  ordered.forEach((section, index) => {
    const column = Math.min(
      columnCount - 1,
      Math.floor(index * columnCount / ordered.length)
    );
    columns[column]!.push(section);
  });
  return columns;
}

function orderColumns(
  columns: ArchitectureSectionSummary[][],
  routes: readonly ArchitectureRouteSummary[]
): void {
  const weights = new Map<string, Map<string, number>>();
  for (const route of routes) {
    addWeight(weights, route.sourceSection, route.targetSection, route.calls);
    addWeight(weights, route.targetSection, route.sourceSection, route.calls);
  }

  for (let pass = 0; pass < 5; pass += 1) {
    for (const direction of [1, -1] as const) {
      const start = direction === 1 ? 1 : columns.length - 2;
      const end = direction === 1 ? columns.length : -1;
      for (let columnIndex = start; columnIndex !== end; columnIndex += direction) {
        const neighbor = columns[columnIndex - direction]!;
        const neighborIndex = new Map(neighbor.map((section, index) => [section.id, index]));
        columns[columnIndex]!.sort((left, right) =>
          barycenter(left.id, weights, neighborIndex)
          - barycenter(right.id, weights, neighborIndex)
          || left.name.localeCompare(right.name)
        );
      }
    }
  }
}

function addWeight(
  weights: Map<string, Map<string, number>>,
  source: string,
  target: string,
  weight: number
): void {
  const targets = weights.get(source) ?? new Map<string, number>();
  targets.set(target, (targets.get(target) ?? 0) + weight);
  weights.set(source, targets);
}

function barycenter(
  sectionId: string,
  weights: ReadonlyMap<string, ReadonlyMap<string, number>>,
  neighborIndex: ReadonlyMap<string, number>
): number {
  let weightedPosition = 0;
  let totalWeight = 0;
  for (const [neighborId, weight] of weights.get(sectionId) ?? []) {
    const index = neighborIndex.get(neighborId);
    if (index === undefined) continue;
    weightedPosition += index * weight;
    totalWeight += weight;
  }
  return totalWeight > 0 ? weightedPosition / totalWeight : Number.MAX_SAFE_INTEGER;
}

function routePath(
  source: ArchitectureLayoutNode,
  target: ArchitectureLayoutNode,
  direction: ArchitectureLayoutRoute["direction"],
  band: number,
  viewport: { width: number; height: number }
): string {
  if (direction === "forward") {
    const startX = source.x + source.width / 2;
    const endX = target.x - target.width / 2;
    const channelX = (startX + endX) / 2 + spread(band % 9, 6);
    return roundedOrthogonalPath(startX, source.y, channelX, target.y, endX);
  }

  if (direction === "backward") {
    const startX = source.x - source.width / 2;
    const endX = target.x + target.width / 2;
    const useTop = (source.y + target.y) / 2 < viewport.height / 2;
    const channelY = useTop
      ? 54 - (band % 3) * 11
      : viewport.height - 42 + (band % 3) * 11;
    const sourceTurnX = startX - 26 - (band % 4) * 7;
    const targetTurnX = endX + 26 + (band % 4) * 7;
    return [
      `M ${startX} ${source.y}`,
      `H ${sourceTurnX}`,
      `Q ${sourceTurnX - 10} ${source.y} ${sourceTurnX - 10} ${source.y + (useTop ? -10 : 10)}`,
      `V ${channelY}`,
      `H ${targetTurnX + 10}`,
      `Q ${targetTurnX} ${channelY} ${targetTurnX} ${channelY + (useTop ? 10 : -10)}`,
      `V ${target.y}`,
      `H ${endX}`
    ].join(" ");
  }

  const routeRight = source.column < 2;
  const edgeX = routeRight
    ? Math.max(source.x + source.width / 2, target.x + target.width / 2)
      + 34 + spread(band % 9, 6)
    : Math.min(source.x - source.width / 2, target.x - target.width / 2)
      - 34 - spread(band % 9, 6);
  const startX = source.x + (routeRight ? source.width / 2 : -source.width / 2);
  const endX = target.x + (routeRight ? target.width / 2 : -target.width / 2);
  return roundedOrthogonalPath(startX, source.y, edgeX, target.y, endX);
}

function roundedOrthogonalPath(
  startX: number,
  startY: number,
  channelX: number,
  endY: number,
  endX: number
): string {
  const verticalDirection = Math.sign(endY - startY) || 1;
  const horizontalDirection = Math.sign(endX - channelX) || 1;
  const radius = Math.min(12, Math.abs(endY - startY) / 2, Math.abs(channelX - startX) / 2);
  return [
    `M ${startX} ${startY}`,
    `H ${channelX - Math.sign(channelX - startX) * radius}`,
    `Q ${channelX} ${startY} ${channelX} ${startY + verticalDirection * radius}`,
    `V ${endY - verticalDirection * radius}`,
    `Q ${channelX} ${endY} ${channelX + horizontalDirection * radius} ${endY}`,
    `H ${endX}`
  ].join(" ");
}

function routeDirection(
  source: ArchitectureLayoutNode,
  target: ArchitectureLayoutNode
): ArchitectureLayoutRoute["direction"] {
  if (target.column > source.column) return "forward";
  if (target.column < source.column) return "backward";
  return "lateral";
}

function flowBias(section: ArchitectureSectionSummary): number {
  const total = section.incomingCalls + section.outgoingCalls;
  return total === 0 ? 0 : (section.outgoingCalls - section.incomingCalls) / total;
}

function nearestColumn(x: number, lanes: readonly ArchitectureLayoutLane[]): number {
  let nearest = 0;
  let distance = Number.POSITIVE_INFINITY;
  for (const lane of lanes) {
    const center = lane.x + lane.width / 2;
    const nextDistance = Math.abs(center - x);
    if (nextDistance < distance) {
      nearest = lane.index;
      distance = nextDistance;
    }
  }
  return nearest;
}

function laneLabel(index: number, count: number): string {
  if (count === 1) return "Subsystem";
  if (index === 0) return "Upstream";
  if (index === count - 1) return "Downstream";
  return `Flow ${index + 1}`;
}

function spread(index: number, distance: number): number {
  if (index === 0) return 0;
  const step = Math.ceil(index / 2);
  return (index % 2 === 0 ? -1 : 1) * step * distance;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}
