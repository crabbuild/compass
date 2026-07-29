import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent
} from "react";
import {
  Maximize2Icon,
  MinusIcon,
  MoveIcon,
  PlusIcon,
  RotateCcwIcon
} from "lucide-react";
import type { ArchitectureOverview } from "../contracts/architecture";
import {
  layoutArchitecture,
  type ArchitectureLayoutNode,
  type ArchitecturePosition
} from "./layout";

export type ArchitectureSelection =
  | { kind: "section"; id: string }
  | { kind: "route"; id: string }
  | undefined;

type DragState = {
  id: string;
  pointerId: number;
  offsetX: number;
  offsetY: number;
  startClientX: number;
  startClientY: number;
  moved: boolean;
};

export function ArchitectureMap({
  overview,
  selection,
  onSelect
}: {
  overview: ArchitectureOverview;
  selection: ArchitectureSelection;
  onSelect(selection: Exclude<ArchitectureSelection, undefined>): void;
}) {
  const storageKey = [
    "compass.architecture.layout.v1",
    overview.provenance.projectName,
    overview.scope,
    overview.evidence
  ].join(":");
  const [positions, setPositions] = useState<Record<string, ArchitecturePosition>>(
    () => loadPositions(storageKey)
  );
  const positionsRef = useRef(positions);
  const layout = useMemo(
    () => layoutArchitecture(overview.sections, overview.routes, undefined, positions),
    [overview.routes, overview.sections, positions]
  );
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [draggingId, setDraggingId] = useState<string>();
  const panDrag = useRef<
    { x: number; y: number; panX: number; panY: number; pointerId: number } | undefined
  >(undefined);
  const nodeDrag = useRef<DragState | undefined>(undefined);

  useEffect(() => {
    const next = loadPositions(storageKey);
    positionsRef.current = next;
    setPositions(next);
    setZoom(1);
    setPan({ x: 0, y: 0 });
  }, [storageKey]);

  const connected = useMemo(() => {
    if (selection?.kind !== "section") return new Set<string>();
    return new Set(
      overview.routes
        .filter((route) =>
          route.sourceSection === selection.id || route.targetSection === selection.id
        )
        .flatMap((route) => [route.id, route.sourceSection, route.targetSection])
    );
  }, [overview.routes, selection]);
  const maximumCalls = Math.max(1, ...overview.routes.map((route) => route.calls));
  const viewWidth = layout.width / zoom;
  const viewHeight = layout.height / zoom;
  const viewBox = `${(layout.width - viewWidth) / 2 + pan.x} ${
    (layout.height - viewHeight) / 2 + pan.y
  } ${viewWidth} ${viewHeight}`;

  const activate = (next: Exclude<ArchitectureSelection, undefined>) => {
    onSelect(next);
  };
  const onRouteKeyDown = (
    event: KeyboardEvent<SVGGElement>,
    next: Exclude<ArchitectureSelection, undefined>
  ) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      activate(next);
    }
  };
  const moveNode = (id: string, position: ArchitecturePosition) => {
    const next = { ...positionsRef.current, [id]: position };
    positionsRef.current = next;
    setPositions(next);
  };
  const savePositions = () => {
    try {
      window.localStorage.setItem(storageKey, JSON.stringify(positionsRef.current));
    } catch {
      // A locked-down webview can disable storage; dragging still works for this session.
    }
  };
  const resetLayout = () => {
    positionsRef.current = {};
    setPositions({});
    try {
      window.localStorage.removeItem(storageKey);
    } catch {
      // Resetting in-memory positions is still sufficient for this session.
    }
  };
  const startNodeDrag = (
    event: PointerEvent<SVGGElement>,
    draggedNode: ArchitectureLayoutNode
  ) => {
    event.stopPropagation();
    const point = svgPoint(event.currentTarget.ownerSVGElement, event);
    if (!point) return;
    nodeDrag.current = {
      id: draggedNode.id,
      pointerId: event.pointerId,
      offsetX: point.x - draggedNode.x,
      offsetY: point.y - draggedNode.y,
      startClientX: event.clientX,
      startClientY: event.clientY,
      moved: false
    };
    setDraggingId(draggedNode.id);
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const finishNodeDrag = (
    event: PointerEvent<SVGGElement>,
    draggedNode: ArchitectureLayoutNode
  ) => {
    const current = nodeDrag.current;
    if (!current || current.id !== draggedNode.id || current.pointerId !== event.pointerId) {
      return;
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    nodeDrag.current = undefined;
    setDraggingId(undefined);
    if (current.moved) savePositions();
    else activate({ kind: "section", id: draggedNode.id });
  };
  const cancelNodeDrag = (event: PointerEvent<SVGGElement>) => {
    if (nodeDrag.current?.pointerId !== event.pointerId) return;
    nodeDrag.current = undefined;
    setDraggingId(undefined);
  };
  const onNodeKeyDown = (
    event: KeyboardEvent<SVGGElement>,
    movedNode: ArchitectureLayoutNode
  ) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      activate({ kind: "section", id: movedNode.id });
      return;
    }
    if (!event.altKey || !event.key.startsWith("Arrow")) return;
    event.preventDefault();
    const distance = event.shiftKey ? 24 : 8;
    moveNode(movedNode.id, {
      x: movedNode.x
        + (event.key === "ArrowRight" ? distance : event.key === "ArrowLeft" ? -distance : 0),
      y: movedNode.y
        + (event.key === "ArrowDown" ? distance : event.key === "ArrowUp" ? -distance : 0)
    });
    savePositions();
  };

  return (
    <section className="architecture-map-panel" aria-label="Interactive system map">
      <div className="architecture-map-toolbar" aria-label="Map controls">
        <button
          type="button"
          aria-label="Zoom out"
          onClick={() => setZoom((value) => Math.max(0.75, value - 0.25))}
        >
          <MinusIcon aria-hidden="true" />
        </button>
        <span>{Math.round(zoom * 100)}%</span>
        <button
          type="button"
          aria-label="Zoom in"
          onClick={() => setZoom((value) => Math.min(2, value + 0.25))}
        >
          <PlusIcon aria-hidden="true" />
        </button>
        <button
          type="button"
          aria-label="Fit architecture map"
          onClick={() => {
            setZoom(1);
            setPan({ x: 0, y: 0 });
          }}
        >
          <Maximize2Icon aria-hidden="true" />
        </button>
        <button
          type="button"
          aria-label="Reset subsystem positions"
          title="Reset subsystem positions"
          disabled={Object.keys(positions).length === 0}
          onClick={resetLayout}
        >
          <RotateCcwIcon aria-hidden="true" />
        </button>
      </div>
      {layout.nodes.length > 0 ? (
        <svg
          className="architecture-map"
          viewBox={viewBox}
          role="img"
          aria-label={`${layout.nodes.length} subsystems and ${layout.routes.length} directed routes`}
          onPointerDown={(event) => {
            if (event.target !== event.currentTarget) return;
            panDrag.current = {
              x: event.clientX,
              y: event.clientY,
              panX: pan.x,
              panY: pan.y,
              pointerId: event.pointerId
            };
            event.currentTarget.setPointerCapture(event.pointerId);
          }}
          onPointerMove={(event) => {
            if (!panDrag.current || panDrag.current.pointerId !== event.pointerId) return;
            const scale = viewWidth / event.currentTarget.getBoundingClientRect().width;
            setPan({
              x: panDrag.current.panX - (event.clientX - panDrag.current.x) * scale,
              y: panDrag.current.panY - (event.clientY - panDrag.current.y) * scale
            });
          }}
          onPointerUp={(event) => {
            if (panDrag.current?.pointerId !== event.pointerId) return;
            panDrag.current = undefined;
            event.currentTarget.releasePointerCapture(event.pointerId);
          }}
          onPointerCancel={() => {
            panDrag.current = undefined;
          }}
        >
          <defs>
            <marker
              id="architecture-arrow"
              className="architecture-arrow architecture-arrow-default"
              markerWidth="8"
              markerHeight="8"
              refX="7"
              refY="4"
              orient="auto"
              markerUnits="userSpaceOnUse"
            >
              <path d="M 0 0 L 8 4 L 0 8 z" />
            </marker>
            <marker
              id="architecture-arrow-incoming"
              className="architecture-arrow architecture-arrow-incoming"
              markerWidth="8"
              markerHeight="8"
              refX="7"
              refY="4"
              orient="auto"
              markerUnits="userSpaceOnUse"
            >
              <path d="M 0 0 L 8 4 L 0 8 z" />
            </marker>
            <marker
              id="architecture-arrow-outgoing"
              className="architecture-arrow architecture-arrow-outgoing"
              markerWidth="8"
              markerHeight="8"
              refX="7"
              refY="4"
              orient="auto"
              markerUnits="userSpaceOnUse"
            >
              <path d="M 0 0 L 8 4 L 0 8 z" />
            </marker>
          </defs>

          <g className="architecture-map-lanes" aria-hidden="true">
            {layout.lanes.map((lane) => (
              <g key={lane.index}>
                <rect
                  x={lane.x + 8}
                  y={34}
                  width={lane.width - 16}
                  height={layout.height - 76}
                  rx="12"
                />
                <text x={lane.x + lane.width / 2} y={61}>{lane.label}</text>
              </g>
            ))}
            <text
              className="architecture-map-direction"
              x={layout.width / 2}
              y={24}
            >
              CALL DIRECTION  →
            </text>
          </g>

          <g className="architecture-routes">
            {layout.routes.map((route) => {
              const selected = selection?.kind === "route" && selection.id === route.id;
              const related = selected
                || selection === undefined
                || (selection.kind === "section" && connected.has(route.id));
              const evidence = route.inferred > route.extracted ? "inferred" : "extracted";
              const focusDirection = selection?.kind === "section"
                ? route.targetSection === selection.id
                  ? "incoming"
                  : route.sourceSection === selection.id
                    ? "outgoing"
                    : undefined
                : undefined;
              const opacity = 0.13 + Math.sqrt(route.calls / maximumCalls) * 0.22;
              return (
                <g
                  key={route.id}
                  role="button"
                  tabIndex={0}
                  aria-label={`${route.sourceSection} to ${route.targetSection}, ${route.calls} calls`}
                  data-direction={route.direction}
                  data-evidence={evidence}
                  data-focus-direction={focusDirection}
                  data-selected={selected || undefined}
                  data-dimmed={!related || undefined}
                  onClick={() => activate({ kind: "route", id: route.id })}
                  onKeyDown={(event) =>
                    onRouteKeyDown(event, { kind: "route", id: route.id })
                  }
                >
                  <path className="architecture-route-hit" d={route.path} />
                  <path
                    className="architecture-route-line"
                    d={route.path}
                    style={{
                      strokeWidth: route.width,
                      "--architecture-route-opacity": opacity
                    } as CSSProperties}
                    markerEnd={
                      focusDirection === "incoming"
                        ? "url(#architecture-arrow-incoming)"
                        : focusDirection === "outgoing"
                          ? "url(#architecture-arrow-outgoing)"
                          : "url(#architecture-arrow)"
                    }
                  />
                  <title>
                    {route.sourceSection} → {route.targetSection}: {route.calls} calls
                  </title>
                </g>
              );
            })}
          </g>

          <g className="architecture-map-nodes">
            {layout.nodes.map((node) => {
              const selected = selection?.kind === "section" && selection.id === node.id;
              const related = selected
                || selection === undefined
                || (selection.kind === "section" && connected.has(node.id))
                || (selection.kind === "route"
                  && overview.routes.some((route) =>
                    route.id === selection.id
                    && (route.sourceSection === node.id || route.targetSection === node.id)
                  ));
              return (
                <g
                  key={node.id}
                  role="button"
                  tabIndex={0}
                  aria-label={`${node.name}, ${node.nodeCount} visible symbols, ${
                    node.incomingCalls
                  } incoming and ${node.outgoingCalls} outgoing calls; drag to reposition`}
                  transform={`translate(${node.x - node.width / 2} ${node.y - node.height / 2})`}
                  data-selected={selected || undefined}
                  data-dimmed={!related || undefined}
                  data-dragging={draggingId === node.id || undefined}
                  onPointerDown={(event) => startNodeDrag(event, node)}
                  onPointerMove={(event) => {
                    const current = nodeDrag.current;
                    if (!current || current.id !== node.id || current.pointerId !== event.pointerId) {
                      return;
                    }
                    const point = svgPoint(event.currentTarget.ownerSVGElement, event);
                    if (!point) return;
                    if (
                      Math.abs(event.clientX - current.startClientX) > 3
                      || Math.abs(event.clientY - current.startClientY) > 3
                    ) {
                      current.moved = true;
                    }
                    moveNode(node.id, {
                      x: point.x - current.offsetX,
                      y: point.y - current.offsetY
                    });
                  }}
                  onPointerUp={(event) => finishNodeDrag(event, node)}
                  onPointerCancel={(event) => cancelNodeDrag(event)}
                  onKeyDown={(event) => onNodeKeyDown(event, node)}
                >
                  <rect className="architecture-map-node-shadow" width={node.width} height={node.height} rx="9" />
                  <rect className="architecture-map-node-card" width={node.width} height={node.height} rx="9" />
                  <rect className="architecture-map-node-accent" width="4" height={node.height - 16} x="8" y="8" rx="2" />
                  <text x="20" y="27" className="architecture-map-node-name">
                    {truncate(node.name, 24)}
                  </text>
                  <text x="20" y="48" className="architecture-map-node-meta">
                    {node.nodeCount.toLocaleString()} symbols
                    {"  ·  "}↓{node.incomingCalls.toLocaleString()}
                    {"  ↑"}{node.outgoingCalls.toLocaleString()}
                  </text>
                  <g className="architecture-map-node-grip" aria-hidden="true">
                    <circle cx={node.width - 18} cy="26" r="1.2" />
                    <circle cx={node.width - 13} cy="26" r="1.2" />
                    <circle cx={node.width - 18} cy="31" r="1.2" />
                    <circle cx={node.width - 13} cy="31" r="1.2" />
                    <circle cx={node.width - 18} cy="36" r="1.2" />
                    <circle cx={node.width - 13} cy="36" r="1.2" />
                  </g>
                  <title>{node.name} — drag to reposition</title>
                </g>
              );
            })}
          </g>
        </svg>
      ) : (
        <div className="architecture-map-empty">
          No subsystems match the current scope and evidence filters.
        </div>
      )}
      <div className="architecture-map-legend" aria-label="Map legend">
        <span><i data-evidence="extracted" /> Extracted</span>
        <span><i data-evidence="inferred" /> Inferred</span>
        <span><i data-direction="incoming" /> Incoming</span>
        <span><i data-direction="outgoing" /> Outgoing</span>
        <span><MoveIcon aria-hidden="true" /> Drag cards to arrange</span>
      </div>
      <details className="architecture-map-table">
        <summary>View routes as a table</summary>
        <table>
          <thead><tr><th>From</th><th>To</th><th>Calls</th><th>Evidence</th></tr></thead>
          <tbody>
            {overview.routes.map((route) => (
              <tr key={route.id}>
                <td>{sectionName(overview, route.sourceSection)}</td>
                <td>{sectionName(overview, route.targetSection)}</td>
                <td>{route.calls.toLocaleString()}</td>
                <td>{route.extracted} extracted · {route.inferred} inferred</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </section>
  );
}

function svgPoint(
  svg: SVGSVGElement | null,
  event: Pick<PointerEvent<SVGGElement>, "clientX" | "clientY">
): ArchitecturePosition | undefined {
  if (!svg) return undefined;
  const matrix = svg.getScreenCTM();
  if (!matrix) return undefined;
  const point = svg.createSVGPoint();
  point.x = event.clientX;
  point.y = event.clientY;
  const transformed = point.matrixTransform(matrix.inverse());
  return { x: transformed.x, y: transformed.y };
}

function loadPositions(key: string): Record<string, ArchitecturePosition> {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(key) ?? "{}") as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return Object.fromEntries(
      Object.entries(parsed).flatMap(([id, value]) => {
        if (
          value
          && typeof value === "object"
          && "x" in value
          && "y" in value
          && typeof value.x === "number"
          && typeof value.y === "number"
          && Number.isFinite(value.x)
          && Number.isFinite(value.y)
        ) {
          return [[id, { x: value.x, y: value.y }]];
        }
        return [];
      })
    );
  } catch {
    return {};
  }
}

function sectionName(overview: ArchitectureOverview, id: string): string {
  return overview.sections.find((section) => section.id === id)?.name ?? id;
}

function truncate(value: string, limit: number): string {
  return value.length <= limit ? value : `${value.slice(0, limit - 1)}…`;
}
