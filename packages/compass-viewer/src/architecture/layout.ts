import type {
  ArchitectureRouteSummary,
  ArchitectureGroupSummary
} from "../contracts/architecture";

export type ArchitecturePosition = {
  x: number;
  y: number;
};

export type ArchitectureLayoutNode = ArchitectureGroupSummary & {
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

const NODE_WIDTH = 224;
const NODE_HEIGHT = 76;
const TOP_GUTTER = 124;
const BOTTOM_GUTTER = 96;
const SIDE_GUTTER = 96;
const MINIMUM_LANE_WIDTH = 288;
const MINIMUM_ROW_STEP = 112;
const MAXIMUM_LANES = 12;

export function layoutArchitecture(
  groups: readonly ArchitectureGroupSummary[],
  routes: readonly ArchitectureRouteSummary[],
  viewport: { width: number; height: number } = { width: 1280, height: 620 },
  positions: Readonly<Record<string, ArchitecturePosition>> = {},
  focusSectionId?: string
): ArchitectureLayout {
  const visible = groups.filter((section) =>
    section.nodeCount > 0 || section.incomingRelationships > 0 || section.outgoingRelationships > 0
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

  const groupIds = new Set(visible.map((section) => section.id));
  const visibleRoutes = routes.filter((route) =>
    groupIds.has(route.sourceGroup) && groupIds.has(route.targetGroup)
  );
  const columns = assignColumns(visible, visibleRoutes);
  const columnCount = columns.length;
  const width = Math.max(
    viewport.width,
    SIDE_GUTTER * 2 + MINIMUM_LANE_WIDTH * columnCount
  );
  const laneWidth = (width - SIDE_GUTTER * 2) / columnCount;
  orderColumns(columns, visibleRoutes);
  const maximumRows = Math.max(...columns.map((column) => column.length));
  const height = Math.max(
    viewport.height,
    TOP_GUTTER + BOTTOM_GUTTER + maximumRows * MINIMUM_ROW_STEP
  );

  const focusColumn = columns.findIndex((column) =>
    column.some((section) => section.id === focusSectionId)
  );
  const lanes = Array.from({ length: columnCount }, (_, index) => ({
    index,
    x: SIDE_GUTTER + laneWidth * index,
    width: laneWidth,
    label: laneLabel(index, columnCount, focusColumn < 0 ? undefined : focusColumn)
  }));
  const availableHeight = height - TOP_GUTTER - BOTTOM_GUTTER;
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
        x: clamp(position.x, SIDE_GUTTER + NODE_WIDTH / 2, width - SIDE_GUTTER - NODE_WIDTH / 2),
        y: clamp(position.y, TOP_GUTTER, height - BOTTOM_GUTTER),
        width: Math.min(NODE_WIDTH, laneWidth - 40),
        height: NODE_HEIGHT,
        column: nearestColumn(position.x, lanes)
      };
    });
  });

  const byId = new Map(nodes.map((node) => [node.id, node]));
  const routeBands = assignRouteBands(visibleRoutes, byId);
  const sourcePorts = assignRoutePorts(visibleRoutes, byId, "source");
  const targetPorts = assignRoutePorts(visibleRoutes, byId, "target");
  const layoutRoutes = visibleRoutes.flatMap((route) => {
    const source = byId.get(route.sourceGroup);
    const target = byId.get(route.targetGroup);
    if (!source || !target) return [];
    const direction = routeDirection(source, target);
    return [{
      ...route,
      direction,
      path: routePath(
        source,
        target,
        direction,
        routeBands.get(route.id) ?? 0,
        { width, height },
        sourcePorts.get(route.id) ?? 0,
        targetPorts.get(route.id) ?? 0
      ),
      width: 1 + Math.min(4.4, Math.log10(route.relationships + 1) * 1.35)
    }];
  });

  return {
    width,
    height,
    nodes,
    routes: layoutRoutes,
    lanes
  };
}

function assignColumns(
  groups: readonly ArchitectureGroupSummary[],
  routes: readonly ArchitectureRouteSummary[]
): ArchitectureGroupSummary[][] {
  if (groups.length === 1) return [[groups[0]!]];
  if (routes.length === 0) return balancedColumns(groups);

  const ordered = feedbackAwareOrder(groups, routes);
  const rank = new Map(ordered.map((section, index) => [section.id, index]));
  const predecessors = new Map<string, ArchitectureRouteSummary[]>();
  for (const section of groups) predecessors.set(section.id, []);
  for (const route of routes) {
    if (
      (rank.get(route.sourceGroup) ?? 0) < (rank.get(route.targetGroup) ?? 0)
      && route.sourceGroup !== route.targetGroup
    ) {
      predecessors.get(route.targetGroup)?.push(route);
    }
  }

  const depth = new Map<string, number>();
  for (const section of ordered) {
    const parentDepths = (predecessors.get(section.id) ?? []).map(
      (route) => (depth.get(route.sourceGroup) ?? 0) + 1
    );
    depth.set(section.id, parentDepths.length > 0 ? Math.max(...parentDepths) : 0);
  }
  const maximumDepth = Math.max(0, ...depth.values());
  const desiredLaneCount = Math.min(MAXIMUM_LANES, maximumDepth + 1);
  if (desiredLaneCount === 1) return balancedColumns(groups);

  const columns = Array.from(
    { length: desiredLaneCount },
    () => [] as ArchitectureGroupSummary[]
  );
  for (const section of ordered) {
    const rawDepth = depth.get(section.id) ?? 0;
    const column = maximumDepth < MAXIMUM_LANES
      ? rawDepth
      : Math.ceil(rawDepth * (MAXIMUM_LANES - 1) / maximumDepth);
    columns[column]!.push(section);
  }
  return columns.filter((column) => column.length > 0);
}

/**
 * Produces a stable ordering for cyclic graphs by peeling true sources and
 * sinks first, then breaking remaining cycles at the node with the strongest
 * weighted outgoing bias. Edges that oppose this order become feedback routes.
 */
function feedbackAwareOrder(
  groups: readonly ArchitectureGroupSummary[],
  routes: readonly ArchitectureRouteSummary[]
): ArchitectureGroupSummary[] {
  const byId = new Map(groups.map((section) => [section.id, section]));
  const remaining = new Set(byId.keys());
  const left: ArchitectureGroupSummary[] = [];
  const right: ArchitectureGroupSummary[] = [];

  while (remaining.size > 0) {
    const weights = weightedDegrees(remaining, routes);
    const candidates = [...remaining].map((id) => ({
      section: byId.get(id)!,
      incoming: weights.incoming.get(id) ?? 0,
      outgoing: weights.outgoing.get(id) ?? 0
    }));
    const sources = candidates.filter((candidate) => candidate.incoming === 0);
    const sinks = candidates.filter((candidate) => candidate.outgoing === 0);
    if (sources.length > 0) {
      const chosen = sources.sort((a, b) =>
        b.outgoing - a.outgoing || compareSections(a.section, b.section)
      )[0]!;
      left.push(chosen.section);
      remaining.delete(chosen.section.id);
      continue;
    }
    if (sinks.length > 0) {
      const chosen = sinks.sort((a, b) =>
        b.incoming - a.incoming || compareSections(a.section, b.section)
      )[0]!;
      right.unshift(chosen.section);
      remaining.delete(chosen.section.id);
      continue;
    }
    const chosen = candidates.sort((a, b) =>
      (b.outgoing - b.incoming) - (a.outgoing - a.incoming)
      || (b.outgoing + b.incoming) - (a.outgoing + a.incoming)
      || compareSections(a.section, b.section)
    )[0]!;
    left.push(chosen.section);
    remaining.delete(chosen.section.id);
  }
  return [...left, ...right];
}

function weightedDegrees(
  remaining: ReadonlySet<string>,
  routes: readonly ArchitectureRouteSummary[]
): {
  incoming: Map<string, number>;
  outgoing: Map<string, number>;
} {
  const incoming = new Map<string, number>();
  const outgoing = new Map<string, number>();
  for (const route of routes) {
    if (
      route.sourceGroup === route.targetGroup
      || !remaining.has(route.sourceGroup)
      || !remaining.has(route.targetGroup)
    ) continue;
    const weight = 1 + Math.log10(route.relationships + 1);
    outgoing.set(route.sourceGroup, (outgoing.get(route.sourceGroup) ?? 0) + weight);
    incoming.set(route.targetGroup, (incoming.get(route.targetGroup) ?? 0) + weight);
  }
  return { incoming, outgoing };
}

function balancedColumns(
  groups: readonly ArchitectureGroupSummary[]
): ArchitectureGroupSummary[][] {
  const columnCount = Math.min(
    6,
    Math.max(2, Math.ceil(Math.sqrt(groups.length)))
  );
  const columns = Array.from(
    { length: columnCount },
    () => [] as ArchitectureGroupSummary[]
  );
  [...groups]
    .sort(compareSections)
    .forEach((section, index) => {
      columns[Math.floor(index * columnCount / groups.length)]!.push(section);
    });
  return columns;
}

function orderColumns(
  columns: ArchitectureGroupSummary[][],
  routes: readonly ArchitectureRouteSummary[]
): void {
  const weights = new Map<string, Map<string, number>>();
  for (const route of routes) {
    addWeight(weights, route.sourceGroup, route.targetGroup, route.relationships);
    addWeight(weights, route.targetGroup, route.sourceGroup, route.relationships);
  }

  for (let pass = 0; pass < 8; pass += 1) {
    for (const direction of [1, -1] as const) {
      const start = direction === 1 ? 1 : columns.length - 2;
      const end = direction === 1 ? columns.length : -1;
      for (let columnIndex = start; columnIndex !== end; columnIndex += direction) {
        const neighborPositions = new Map<string, number>();
        const neighborColumns = direction === 1
          ? columns.slice(0, columnIndex)
          : columns.slice(columnIndex + 1);
        for (const neighbor of neighborColumns) {
          neighbor.forEach((section, index) => {
            neighborPositions.set(
              section.id,
              neighbor.length === 1 ? 0.5 : index / (neighbor.length - 1)
            );
          });
        }
        const previous = new Map(
          columns[columnIndex]!.map((section, index, column) => [
            section.id,
            column.length === 1 ? 0.5 : index / (column.length - 1)
          ])
        );
        columns[columnIndex]!.sort((left, right) => {
          const leftPosition = barycenter(left.id, weights, neighborPositions);
          const rightPosition = barycenter(right.id, weights, neighborPositions);
          return (leftPosition ?? previous.get(left.id) ?? 0.5)
            - (rightPosition ?? previous.get(right.id) ?? 0.5)
            || compareSections(left, right);
        });
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
  groupId: string,
  weights: ReadonlyMap<string, ReadonlyMap<string, number>>,
  neighborIndex: ReadonlyMap<string, number>
): number | undefined {
  let weightedPosition = 0;
  let totalWeight = 0;
  for (const [neighborId, weight] of weights.get(groupId) ?? []) {
    const index = neighborIndex.get(neighborId);
    if (index === undefined) continue;
    weightedPosition += index * weight;
    totalWeight += weight;
  }
  return totalWeight > 0 ? weightedPosition / totalWeight : undefined;
}

function routePath(
  source: ArchitectureLayoutNode,
  target: ArchitectureLayoutNode,
  direction: ArchitectureLayoutRoute["direction"],
  band: number,
  viewport: { width: number; height: number },
  sourcePortOffset: number,
  targetPortOffset: number
): string {
  const sourceY = source.y + sourcePortOffset;
  const targetY = target.y + targetPortOffset;
  if (direction === "forward") {
    const startX = source.x + source.width / 2;
    const endX = target.x - target.width / 2;
    const horizontalDistance = Math.max(1, endX - startX);
    const curve = clamp(horizontalDistance * 0.42, 42, 168);
    const separation = spread(band % 9, 1.5);
    return [
      `M ${startX} ${sourceY}`,
      `C ${startX + curve} ${sourceY + separation}`,
      `${endX - curve} ${targetY + separation}`,
      `${endX} ${targetY}`
    ].join(" ");
  }

  if (direction === "backward") {
    const startX = source.x - source.width / 2;
    const endX = target.x + target.width / 2;
    const useTop = (sourceY + targetY) / 2 < viewport.height / 2;
    const channelY = useTop
      ? 74 - (band % 4) * 10
      : viewport.height - 58 + (band % 4) * 10;
    const sourceTurnX = startX - 24 - (band % 4) * 6;
    const targetTurnX = endX + 24 + (band % 4) * 6;
    return [
      `M ${startX} ${sourceY}`,
      `H ${sourceTurnX}`,
      `Q ${sourceTurnX - 10} ${sourceY} ${sourceTurnX - 10} ${sourceY + (useTop ? -10 : 10)}`,
      `V ${channelY}`,
      `H ${targetTurnX + 10}`,
      `Q ${targetTurnX} ${channelY} ${targetTurnX} ${channelY + (useTop ? 10 : -10)}`,
      `V ${targetY}`,
      `H ${endX}`
    ].join(" ");
  }

  const routeRight = source.column % 2 === 0;
  const localOffset = 24 + (band % 6) * 7;
  const edgeX = routeRight
    ? Math.max(source.x + source.width / 2, target.x + target.width / 2) + localOffset
    : Math.min(source.x - source.width / 2, target.x - target.width / 2) - localOffset;
  const startX = source.x + (routeRight ? source.width / 2 : -source.width / 2);
  const endX = target.x + (routeRight ? target.width / 2 : -target.width / 2);
  return roundedOrthogonalPath(startX, sourceY, edgeX, targetY, endX);
}

function assignRouteBands(
  routes: readonly ArchitectureRouteSummary[],
  nodes: ReadonlyMap<string, ArchitectureLayoutNode>
): Map<string, number> {
  const bands = new Map<string, number>();
  const nextBand = new Map<string, number>();
  const ordered = [...routes].sort((left, right) => {
    const leftSource = nodes.get(left.sourceGroup);
    const rightSource = nodes.get(right.sourceGroup);
    const leftTarget = nodes.get(left.targetGroup);
    const rightTarget = nodes.get(right.targetGroup);
    return (leftSource?.column ?? 0) - (rightSource?.column ?? 0)
      || (leftTarget?.column ?? 0) - (rightTarget?.column ?? 0)
      || (leftSource?.y ?? 0) - (rightSource?.y ?? 0)
      || (leftTarget?.y ?? 0) - (rightTarget?.y ?? 0)
      || right.relationships - left.relationships
      || left.id.localeCompare(right.id);
  });
  for (const route of ordered) {
    const source = nodes.get(route.sourceGroup);
    const target = nodes.get(route.targetGroup);
    if (!source || !target) continue;
    const direction = routeDirection(source, target);
    const key = `${Math.min(source.column, target.column)}:${Math.max(source.column, target.column)}:${direction}`;
    const band = nextBand.get(key) ?? 0;
    bands.set(route.id, band);
    nextBand.set(key, band + 1);
  }
  return bands;
}

function assignRoutePorts(
  routes: readonly ArchitectureRouteSummary[],
  nodes: ReadonlyMap<string, ArchitectureLayoutNode>,
  endpoint: "source" | "target"
): Map<string, number> {
  const grouped = new Map<string, ArchitectureRouteSummary[]>();
  for (const route of routes) {
    const id = endpoint === "source" ? route.sourceGroup : route.targetGroup;
    const group = grouped.get(id) ?? [];
    group.push(route);
    grouped.set(id, group);
  }
  const offsets = new Map<string, number>();
  for (const group of grouped.values()) {
    group.sort((left, right) => {
      const leftOther = nodes.get(
        endpoint === "source" ? left.targetGroup : left.sourceGroup
      );
      const rightOther = nodes.get(
        endpoint === "source" ? right.targetGroup : right.sourceGroup
      );
      return (leftOther?.y ?? 0) - (rightOther?.y ?? 0)
        || (leftOther?.x ?? 0) - (rightOther?.x ?? 0)
        || left.id.localeCompare(right.id);
    });
    const spacing = Math.min(8, 38 / Math.max(1, group.length - 1));
    group.forEach((route, index) => {
      offsets.set(route.id, (index - (group.length - 1) / 2) * spacing);
    });
  }
  return offsets;
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

function laneLabel(index: number, count: number, focus?: number): string {
  if (focus !== undefined) {
    if (index === focus) return "Focus";
    const distance = Math.abs(index - focus);
    if (index < focus) return distance === 1 ? "Direct callers" : `Caller depth ${distance}`;
    return distance === 1 ? "Direct dependencies" : `Dependency depth ${distance}`;
  }
  if (count === 1) return "Subsystem";
  if (index === 0) return "Callers";
  if (index === count - 1) return "Dependencies";
  return `Depth ${index}`;
}

function compareSections(
  left: ArchitectureGroupSummary,
  right: ArchitectureGroupSummary
): number {
  return (right.incomingRelationships + right.outgoingRelationships)
    - (left.incomingRelationships + left.outgoingRelationships)
    || left.name.localeCompare(right.name)
    || left.id.localeCompare(right.id);
}

function spread(index: number, distance: number): number {
  if (index === 0) return 0;
  const step = Math.ceil(index / 2);
  return (index % 2 === 0 ? -1 : 1) * step * distance;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}
