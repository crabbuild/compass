import { useMemo, type MouseEvent } from "react";
import type { GraphViewModel } from "../contracts/graph";
import { nodeSemanticCategory, nodeSemanticCssColor } from "./semanticAppearance";

const WIDTH = 176;
const HEIGHT = 108;
const PADDING = 8;
const MINIMAP_NODE_LIMIT = 1_500;
const MINIMAP_EDGE_LIMIT = 2_000;

export type GraphMinimapSnapshot = {
  positions: ReadonlyMap<string, { x: number; y: number }>;
  viewport: { left: number; top: number; width: number; height: number };
};

type MinimapGeometry = {
  project(position: { x: number; y: number }): { x: number; y: number };
  unproject(position: { x: number; y: number }): { x: number; y: number };
  viewport: { x: number; y: number; width: number; height: number };
};

export function graphMinimapGeometry(snapshot: GraphMinimapSnapshot): MinimapGeometry {
  const positions = [...snapshot.positions.values()];
  const viewportRight = snapshot.viewport.left + snapshot.viewport.width;
  const viewportBottom = snapshot.viewport.top + snapshot.viewport.height;
  const xs = [
    ...positions.map((position) => position.x),
    snapshot.viewport.left,
    viewportRight
  ];
  const ys = [
    ...positions.map((position) => position.y),
    snapshot.viewport.top,
    viewportBottom
  ];
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  const spanX = Math.max(1, maxX - minX);
  const spanY = Math.max(1, maxY - minY);
  const drawableWidth = WIDTH - PADDING * 2;
  const drawableHeight = HEIGHT - PADDING * 2;
  const scale = Math.min(drawableWidth / spanX, drawableHeight / spanY);
  const renderedWidth = spanX * scale;
  const renderedHeight = spanY * scale;
  const offsetX = (WIDTH - renderedWidth) / 2;
  const offsetY = (HEIGHT - renderedHeight) / 2;
  const project = ({ x, y }: { x: number; y: number }) => ({
    x: offsetX + (x - minX) * scale,
    y: offsetY + (y - minY) * scale
  });
  const unproject = ({ x, y }: { x: number; y: number }) => ({
    x: minX + (x - offsetX) / scale,
    y: minY + (y - offsetY) / scale
  });
  const topLeft = project({ x: snapshot.viewport.left, y: snapshot.viewport.top });
  const bottomRight = project({ x: viewportRight, y: viewportBottom });
  return {
    project,
    unproject,
    viewport: {
      x: topLeft.x,
      y: topLeft.y,
      width: bottomRight.x - topLeft.x,
      height: bottomRight.y - topLeft.y
    }
  };
}

export function GraphMinimap({
  model,
  snapshot,
  visibleNodeIds,
  visibleEdgeIds,
  focusedNodeId,
  semanticDetail = false,
  onNavigate
}: {
  model: GraphViewModel;
  snapshot: GraphMinimapSnapshot;
  visibleNodeIds?: ReadonlySet<string> | undefined;
  visibleEdgeIds?: ReadonlySet<string> | undefined;
  focusedNodeId: string | null;
  semanticDetail?: boolean;
  onNavigate(position: { x: number; y: number }): void;
}) {
  const geometry = useMemo(() => graphMinimapGeometry(snapshot), [snapshot]);
  const visibleNodes = useMemo(() => model.nodes
    .filter((node) => !visibleNodeIds || visibleNodeIds.has(node.id))
    .sort((left, right) => left.id.localeCompare(right.id)), [model.nodes, visibleNodeIds]);
  const nodes = visibleNodes.slice(0, MINIMAP_NODE_LIMIT);
  const edges = useMemo(() => model.edges
    .filter((edge) => (!visibleEdgeIds || visibleEdgeIds.has(edge.id))
      && (!visibleNodeIds
        || (visibleNodeIds.has(edge.source) && visibleNodeIds.has(edge.target))))
    .sort((left, right) => left.id.localeCompare(right.id))
    .slice(0, MINIMAP_EDGE_LIMIT), [model.edges, visibleEdgeIds, visibleNodeIds]);
  const communityColors = useMemo(
    () => new Map(model.communities.map((community) => [community.id, community.color])),
    [model.communities]
  );
  const handleClick = (event: MouseEvent<HTMLButtonElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    if (bounds.width <= 0 || bounds.height <= 0) return;
    onNavigate(geometry.unproject({
      x: (event.clientX - bounds.left) / bounds.width * WIDTH,
      y: (event.clientY - bounds.top) / bounds.height * HEIGHT
    }));
  };
  return (
    <aside className="compass-minimap compass-glass-panel" aria-label="Graph overview">
      <header>
        <span>Map</span>
        <small>{nodes.length < visibleNodes.length
          ? `${nodes.length.toLocaleString()} / ${visibleNodes.length.toLocaleString()}`
          : `${nodes.length.toLocaleString()} nodes`}</small>
      </header>
      <button
        type="button"
        aria-label="Graph minimap. Click to reposition the viewport"
        onClick={handleClick}
      >
        <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} aria-hidden="true">
          <g className="compass-minimap-edges">
            {edges.map((edge) => {
              const source = snapshot.positions.get(edge.source);
              const target = snapshot.positions.get(edge.target);
              if (!source || !target) return null;
              const from = geometry.project(source);
              const to = geometry.project(target);
              return <line key={edge.id} x1={from.x} y1={from.y} x2={to.x} y2={to.y} />;
            })}
          </g>
          <g className="compass-minimap-nodes">
            {nodes.map((node) => {
              const position = snapshot.positions.get(node.id);
              if (!position) return null;
              const point = geometry.project(position);
              return (
                <circle
                  key={node.id}
                  cx={point.x}
                  cy={point.y}
                  r={node.id === focusedNodeId ? 3 : 1.65}
                  fill={semanticDetail
                    ? nodeSemanticCssColor(nodeSemanticCategory(node.kind))
                    : node.color?.background
                      ?? communityColors.get(node.community)
                      ?? "currentColor"}
                  data-node-category={semanticDetail
                    ? nodeSemanticCategory(node.kind)
                    : undefined}
                  data-focused={node.id === focusedNodeId ? "true" : undefined}
                />
              );
            })}
          </g>
          <rect
            className="compass-minimap-viewport"
            x={geometry.viewport.x}
            y={geometry.viewport.y}
            width={geometry.viewport.width}
            height={geometry.viewport.height}
            rx="2"
          />
        </svg>
      </button>
    </aside>
  );
}
