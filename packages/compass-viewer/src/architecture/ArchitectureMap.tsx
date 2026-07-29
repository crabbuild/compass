import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Maximize2Icon, MinusIcon, PlusIcon } from "lucide-react";
import type { ArchitectureOverview } from "../contracts/architecture";
import { layoutArchitecture } from "./layout";

export type ArchitectureSelection =
  | { kind: "section"; id: string }
  | { kind: "route"; id: string }
  | undefined;

export function ArchitectureMap({
  overview,
  selection,
  onSelect
}: {
  overview: ArchitectureOverview;
  selection: ArchitectureSelection;
  onSelect(selection: Exclude<ArchitectureSelection, undefined>): void;
}) {
  const layout = useMemo(
    () => layoutArchitecture(overview.sections, overview.routes),
    [overview.routes, overview.sections]
  );
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const drag = useRef<{ x: number; y: number; panX: number; panY: number } | undefined>(
    undefined
  );
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
  const viewWidth = layout.width / zoom;
  const viewHeight = layout.height / zoom;
  const viewBox = `${(layout.width - viewWidth) / 2 + pan.x} ${
    (layout.height - viewHeight) / 2 + pan.y
  } ${viewWidth} ${viewHeight}`;

  const activate = (next: Exclude<ArchitectureSelection, undefined>) => {
    onSelect(next);
  };
  const onKeyDown = (
    event: KeyboardEvent<SVGGElement>,
    next: Exclude<ArchitectureSelection, undefined>
  ) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      activate(next);
    }
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
      </div>
      {layout.nodes.length > 0 ? (
        <svg
          className="architecture-map"
          viewBox={viewBox}
          role="img"
          aria-label={`${layout.nodes.length} subsystems and ${layout.routes.length} directed routes`}
          onPointerDown={(event) => {
            if (event.target !== event.currentTarget) return;
            drag.current = { x: event.clientX, y: event.clientY, panX: pan.x, panY: pan.y };
            event.currentTarget.setPointerCapture(event.pointerId);
          }}
          onPointerMove={(event) => {
            if (!drag.current) return;
            const scale = viewWidth / event.currentTarget.getBoundingClientRect().width;
            setPan({
              x: drag.current.panX - (event.clientX - drag.current.x) * scale,
              y: drag.current.panY - (event.clientY - drag.current.y) * scale
            });
          }}
          onPointerUp={(event) => {
            drag.current = undefined;
            event.currentTarget.releasePointerCapture(event.pointerId);
          }}
        >
          <defs>
            <marker
              id="architecture-arrow"
              markerWidth="8"
              markerHeight="8"
              refX="7"
              refY="4"
              orient="auto"
              markerUnits="strokeWidth"
            >
              <path d="M 0 0 L 8 4 L 0 8 z" />
            </marker>
          </defs>
          <g className="architecture-routes">
            {layout.routes.map((route) => {
              const selected = selection?.kind === "route" && selection.id === route.id;
              const related = selected
                || selection === undefined
                || (selection.kind === "section" && connected.has(route.id));
              const evidence = route.inferred > route.extracted ? "inferred" : "extracted";
              return (
                <g
                  key={route.id}
                  role="button"
                  tabIndex={0}
                  aria-label={`${route.sourceSection} to ${route.targetSection}, ${route.calls} calls`}
                  data-evidence={evidence}
                  data-selected={selected || undefined}
                  data-dimmed={!related || undefined}
                  onClick={() => activate({ kind: "route", id: route.id })}
                  onKeyDown={(event) => onKeyDown(event, { kind: "route", id: route.id })}
                >
                  <path className="architecture-route-hit" d={route.path} />
                  <path
                    className="architecture-route-line"
                    d={route.path}
                    style={{ strokeWidth: route.width }}
                    markerEnd="url(#architecture-arrow)"
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
                  } incoming and ${node.outgoingCalls} outgoing calls`}
                  transform={`translate(${node.x - node.width / 2} ${node.y - node.height / 2})`}
                  data-selected={selected || undefined}
                  data-dimmed={!related || undefined}
                  onClick={() => activate({ kind: "section", id: node.id })}
                  onKeyDown={(event) => onKeyDown(event, { kind: "section", id: node.id })}
                >
                  <rect width={node.width} height={node.height} rx="8" />
                  <text x="14" y="24" className="architecture-map-node-name">
                    {truncate(node.name, 22)}
                  </text>
                  <text x="14" y="44" className="architecture-map-node-meta">
                    {node.nodeCount.toLocaleString()} symbols ·{" "}
                    {(node.incomingCalls + node.outgoingCalls).toLocaleString()} routed calls
                  </text>
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
        <span><i data-evidence="extracted" /> Extracted-majority</span>
        <span><i data-evidence="inferred" /> Inferred-majority</span>
        <span>Line width = call volume</span>
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

function sectionName(overview: ArchitectureOverview, id: string): string {
  return overview.sections.find((section) => section.id === id)?.name ?? id;
}

function truncate(value: string, limit: number): string {
  return value.length <= limit ? value : `${value.slice(0, limit - 1)}…`;
}
