import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
  type UIEvent
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

type RouteMode = "key" | "complete";
type ScrollPosition = "start" | "middle" | "end" | "none";
type ViewportSize = { width: number; height: number };
type ArchitectureRoute = ArchitectureOverview["routes"][number];

const KEY_ROUTE_LIMIT = 16;
const DEFAULT_VIEWPORT_SIZE: ViewportSize = { width: 1280, height: 620 };

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
    "compass.architecture.layout.v2",
    overview.provenance.projectName,
    overview.scope,
    overview.evidence
  ].join(":");
  const [positions, setPositions] = useState<Record<string, ArchitecturePosition>>(
    () => loadPositions(storageKey)
  );
  const positionsRef = useRef(positions);
  const [zoom, setZoom] = useState(1);
  const [scrollPosition, setScrollPosition] = useState<ScrollPosition>("none");
  const scrollPositionRef = useRef<ScrollPosition>("none");
  const maximumScrollRef = useRef(0);
  const [viewportSize, setViewportSize] = useState(DEFAULT_VIEWPORT_SIZE);
  const [routeMode, setRouteMode] = useState<RouteMode>("key");
  const [draggingId, setDraggingId] = useState<string>();
  const viewportRef = useRef<HTMLDivElement>(null);
  const canvasDrag = useRef<
    {
      x: number;
      y: number;
      scrollLeft: number;
      scrollTop: number;
      pointerId: number;
    } | undefined
  >(undefined);
  const nodeDrag = useRef<DragState | undefined>(undefined);

  useEffect(() => {
    const next = loadPositions(storageKey);
    positionsRef.current = next;
    setPositions(next);
    setZoom(1);
    scrollPositionRef.current = "start";
    maximumScrollRef.current = 0;
    setScrollPosition("start");
    viewportRef.current?.scrollTo?.({ left: 0, top: 0 });
    setRouteMode("key");
  }, [storageKey]);

  const keyRouteIds = useMemo(
    () => selectKeyRoutes(overview.routes, selection),
    [overview.routes, selection]
  );
  const routeDisplay = useMemo(
    () => routeMode === "complete"
      ? { routes: overview.routes, reciprocals: new Map<string, ArchitectureRoute>() }
      : collapseReciprocalRoutes(
        overview.routes.filter((route) => keyRouteIds.has(route.id)),
        selection?.kind === "route" ? selection.id : undefined
      ),
    [keyRouteIds, overview.routes, routeMode, selection]
  );
  const routeSummaries = routeDisplay.routes;
  const displayedSections = useMemo(() => {
    if (routeMode === "complete" || selection === undefined) return overview.sections;
    const ids = new Set<string>();
    if (selection.kind === "section") ids.add(selection.id);
    for (const route of routeSummaries) {
      ids.add(route.sourceSection);
      ids.add(route.targetSection);
    }
    return overview.sections.filter((section) => ids.has(section.id));
  }, [overview.sections, routeMode, routeSummaries, selection]);
  const layout = useMemo(
    () => layoutArchitecture(
      displayedSections,
      routeSummaries,
      viewportSize,
      positions,
      routeMode === "key" && selection?.kind === "section" ? selection.id : undefined
    ),
    [displayedSections, positions, routeMode, routeSummaries, selection, viewportSize]
  );
  const displayedRoutes = layout.routes;
  const connected = useMemo(() => {
    if (selection?.kind !== "section") return new Set<string>();
    return new Set(
      displayedRoutes
        .filter((route) =>
          route.sourceSection === selection.id || route.targetSection === selection.id
        )
        .flatMap((route) => [route.id, route.sourceSection, route.targetSection])
    );
  }, [displayedRoutes, selection]);
  const maximumCalls = Math.max(1, ...displayedRoutes.map((route) => route.calls));
  const hiddenRouteCount = overview.routes.length - displayedRoutes.length;
  const canvasSize = {
    width: Math.round(layout.width * zoom),
    height: Math.round(layout.height * zoom)
  };

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
  const resetView = () => {
    setZoom(1);
    scrollPositionRef.current = "start";
    maximumScrollRef.current = 0;
    setScrollPosition("start");
    const reduceMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    viewportRef.current?.scrollTo?.({
      left: 0,
      top: 0,
      behavior: reduceMotion ? "auto" : "smooth"
    });
  };
  const updateScrollPosition = (target: HTMLDivElement) => {
    const maximum = Math.max(0, target.scrollWidth - target.clientWidth);
    const next: ScrollPosition = maximum <= 1
      ? "none"
      : target.scrollLeft <= 1
        ? "start"
        : target.scrollLeft >= maximum - 1
          ? "end"
          : "middle";
    maximumScrollRef.current = maximum;
    scrollPositionRef.current = next;
    setScrollPosition((current) => current === next ? current : next);
  };
  const onViewportScroll = (event: UIEvent<HTMLDivElement>) => {
    updateScrollPosition(event.currentTarget);
  };
  const startCanvasDrag = (event: PointerEvent<SVGSVGElement>) => {
    if (event.target !== event.currentTarget || !viewportRef.current) return;
    canvasDrag.current = {
      x: event.clientX,
      y: event.clientY,
      scrollLeft: viewportRef.current.scrollLeft,
      scrollTop: viewportRef.current.scrollTop,
      pointerId: event.pointerId
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const moveCanvas = (event: PointerEvent<SVGSVGElement>) => {
    const current = canvasDrag.current;
    const viewport = viewportRef.current;
    if (!current || !viewport || current.pointerId !== event.pointerId) return;
    viewport.scrollLeft = current.scrollLeft - (event.clientX - current.x);
    viewport.scrollTop = current.scrollTop - (event.clientY - current.y);
  };
  const finishCanvasDrag = (event: PointerEvent<SVGSVGElement>) => {
    if (canvasDrag.current?.pointerId !== event.pointerId) return;
    canvasDrag.current = undefined;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
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

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const update = () => {
      const previousMaximum = maximumScrollRef.current;
      const maximum = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
      const reachedPreviousEnd = previousMaximum > 1
        && viewport.scrollLeft >= previousMaximum - 1;
      if (
        maximum > previousMaximum
        && (scrollPositionRef.current === "end" || reachedPreviousEnd)
      ) {
        viewport.scrollLeft = maximum;
      }
      updateScrollPosition(viewport);
      if (viewport.clientWidth <= 0 || viewport.clientHeight <= 0) return;
      const measured = {
        width: Math.round(viewport.clientWidth),
        height: Math.round(viewport.clientHeight)
      };
      setViewportSize((current) =>
        current.width === measured.width && current.height === measured.height
          ? current
          : measured
      );
    };
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [canvasSize.height, canvasSize.width]);

  return (
    <section
      className="architecture-map-panel"
      aria-labelledby="architecture-map-title"
      data-route-mode={routeMode}
    >
      <header className="architecture-map-header">
        <div className="architecture-map-intro">
          <span>Flow canvas</span>
          <strong id="architecture-map-title">Subsystem call direction</strong>
          <small id="architecture-map-help">
            Scroll sideways or drag open canvas space to follow the complete architecture.
          </small>
        </div>
        <div className="architecture-map-actions">
          <div className="architecture-route-mode" role="group" aria-label="Route visibility">
            <button
              type="button"
              aria-pressed={routeMode === "key"}
              onClick={() => setRouteMode("key")}
            >
              {selection?.kind === "section" ? "Neighbors" : "Key routes"}
            </button>
            <button
              type="button"
              aria-pressed={routeMode === "complete"}
              onClick={() => setRouteMode("complete")}
            >
              All routes · {overview.routes.length}
            </button>
          </div>
          <div className="architecture-map-toolbar" aria-label="Map controls">
            <button
              type="button"
              aria-label="Zoom out"
              onClick={() => setZoom((value) => Math.max(0.75, value - 0.25))}
            >
              <MinusIcon aria-hidden="true" />
            </button>
            <span aria-live="polite">{Math.round(zoom * 100)}%</span>
            <button
              type="button"
              aria-label="Zoom in"
              onClick={() => setZoom((value) => Math.min(2, value + 0.25))}
            >
              <PlusIcon aria-hidden="true" />
            </button>
            <button
              type="button"
              aria-label="Reset zoom and scroll position"
              title="Reset zoom and scroll position"
              onClick={resetView}
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
        </div>
      </header>
      <div
        ref={viewportRef}
        className="architecture-map-viewport"
        role="region"
        aria-label="Scrollable architecture flow diagram"
        aria-describedby="architecture-map-help"
        tabIndex={0}
        data-scroll-position={scrollPosition}
        onScroll={onViewportScroll}
      >
        {layout.nodes.length > 0 ? (
          <div
            className="architecture-map-canvas"
            style={{ width: canvasSize.width, height: canvasSize.height }}
          >
          <svg
            className="architecture-map"
            viewBox={`0 0 ${layout.width} ${layout.height}`}
            role="group"
            aria-label={mapLabel(
              layout.nodes.length,
              displayedRoutes.length,
              overview.routes.length
            )}
            onPointerDown={startCanvasDrag}
            onPointerMove={moveCanvas}
            onPointerUp={finishCanvasDrag}
            onPointerCancel={() => {
              canvasDrag.current = undefined;
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
              orient="auto-start-reverse"
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
              orient="auto-start-reverse"
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
              orient="auto-start-reverse"
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
              x={96}
              y={24}
            >
              PRIMARY CALL DIRECTION  →
            </text>
          </g>

          <g className="architecture-routes">
            {displayedRoutes.map((route) => {
              const reciprocal = routeDisplay.reciprocals.get(route.id);
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
              const opacity = routeMode === "key"
                ? 0.3 + Math.sqrt(route.calls / maximumCalls) * 0.28
                : 0.1 + Math.sqrt(route.calls / maximumCalls) * 0.2;
              return (
                <g
                  key={route.id}
                  role="button"
                  tabIndex={0}
                  aria-label={`${route.sourceSection} to ${route.targetSection}, ${route.calls} calls${
                    reciprocal ? `; bidirectional, ${reciprocal.calls} reverse calls` : ""
                  }${route.direction === "backward" ? ", feedback route" : ""}`}
                  data-direction={route.direction}
                  data-evidence={evidence}
                  data-reciprocal={reciprocal ? "true" : undefined}
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
                    markerStart={reciprocal ? reverseMarker(focusDirection) : undefined}
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
                    {reciprocal && (
                      ` · ${reciprocal.sourceSection} → ${reciprocal.targetSection}: ${
                        reciprocal.calls
                      } calls`
                    )}
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
          </div>
        ) : (
          <div className="architecture-map-empty">
            <strong>No architecture to draw</strong>
            <span>No subsystems match the current scope and evidence filters.</span>
          </div>
        )}
      </div>
      <footer className="architecture-map-footer">
        <div className="architecture-map-legend" aria-label="Map legend">
          <span><i data-evidence="extracted" /> Extracted</span>
          <span><i data-evidence="inferred" /> Inferred</span>
          <span><i data-direction="incoming" /> Incoming</span>
          <span><i data-direction="outgoing" /> Outgoing</span>
          <span><i data-direction="bidirectional" aria-hidden="true">↔</i> Bidirectional</span>
          <span><i data-direction="feedback" /> Feedback</span>
          <span><MoveIcon aria-hidden="true" /> Drag cards to arrange</span>
          {hiddenRouteCount > 0 && (
            <button type="button" onClick={() => setRouteMode("complete")}>
              {displayedRoutes.length} of {overview.routes.length} routes · Show all
            </button>
          )}
        </div>
        <details className="architecture-map-table">
          <summary>View routes as a table</summary>
          <div>
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
          </div>
        </details>
      </footer>
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

function selectKeyRoutes(
  routes: readonly ArchitectureOverview["routes"][number][],
  selection: ArchitectureSelection
): Set<string> {
  const strongest = (candidates: readonly ArchitectureOverview["routes"][number][]) =>
    [...candidates].sort((left, right) =>
      right.calls - left.calls || left.id.localeCompare(right.id)
    );
  const selected = new Set<string>();
  if (selection?.kind === "route") {
    const selectedRoute = routes.find((route) => route.id === selection.id);
    if (selectedRoute) {
      selected.add(selectedRoute.id);
      const reverse = routes.find((route) =>
        route.sourceSection === selectedRoute.targetSection
        && route.targetSection === selectedRoute.sourceSection
      );
      if (reverse) selected.add(reverse.id);
    }
    return selected;
  } else if (selection?.kind === "section") {
    const incoming = strongest(
      routes.filter((route) => route.targetSection === selection.id)
    ).slice(0, 6);
    const outgoing = strongest(
      routes.filter((route) => route.sourceSection === selection.id)
    ).slice(0, 6);
    for (const route of strongest([...incoming, ...outgoing])) {
      selected.add(route.id);
    }
    return selected;
  }
  for (const route of strongest(routes)) {
    if (selected.size >= KEY_ROUTE_LIMIT) break;
    selected.add(route.id);
  }
  return selected;
}

function collapseReciprocalRoutes(
  routes: readonly ArchitectureRoute[],
  preferredRouteId?: string
): { routes: ArchitectureRoute[]; reciprocals: Map<string, ArchitectureRoute> } {
  const byDirection = new Map(
    routes.map((route) => [routePairKey(route.sourceSection, route.targetSection), route])
  );
  const consumed = new Set<string>();
  const collapsed: ArchitectureRoute[] = [];
  const reciprocals = new Map<string, ArchitectureRoute>();

  for (const route of routes) {
    if (consumed.has(route.id)) continue;
    const reverse = byDirection.get(
      routePairKey(route.targetSection, route.sourceSection)
    );
    if (!reverse || reverse.id === route.id || consumed.has(reverse.id)) {
      collapsed.push(route);
      consumed.add(route.id);
      continue;
    }
    const primary = route.id === preferredRouteId
      ? route
      : reverse.id === preferredRouteId
        ? reverse
        : route.calls > reverse.calls
          || (route.calls === reverse.calls && route.id.localeCompare(reverse.id) <= 0)
          ? route
          : reverse;
    const reciprocal = primary.id === route.id ? reverse : route;
    collapsed.push(primary);
    reciprocals.set(primary.id, reciprocal);
    consumed.add(route.id);
    consumed.add(reverse.id);
  }
  collapsed.sort((left, right) => right.calls - left.calls || left.id.localeCompare(right.id));
  return { routes: collapsed, reciprocals };
}

function routePairKey(source: string, target: string): string {
  return `${source}\u0000${target}`;
}

function reverseMarker(direction: "incoming" | "outgoing" | undefined): string {
  if (direction === "incoming") return "url(#architecture-arrow-outgoing)";
  if (direction === "outgoing") return "url(#architecture-arrow-incoming)";
  return "url(#architecture-arrow)";
}

function mapLabel(nodes: number, displayedRoutes: number, totalRoutes: number): string {
  if (displayedRoutes === totalRoutes) {
    return `${nodes} subsystems and ${totalRoutes} directed routes`;
  }
  return `${nodes} subsystems and ${displayedRoutes} of ${totalRoutes} directed routes visible`;
}
