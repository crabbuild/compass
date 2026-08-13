import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode
} from "react";
import type { GraphViewModel, SourceLocation } from "../contracts/graph";
import type { CodeQueryResponse } from "../contracts/codeQuery";
import { GraphInspector } from "./GraphInspector";
import { GraphTransitionScreen } from "./GraphTransitionScreen";
import { GraphToolbar } from "./GraphToolbar";
import { InspectorResizeHandle } from "./InspectorResizeHandle";
import {
  normalizeInspectorLayout,
  type InspectorLayout
} from "./inspectorLayout";
import { graphNodeActivation } from "./nodeActivation";
import { NodeHoverCard, type GraphHover } from "./NodeHoverCard";
import { EdgeHoverCard, type GraphEdgeHover } from "./EdgeHoverCard";
import { navigableRelationshipSource } from "./sourceNavigation";
import type { GraphSourceRevisions } from "./ChangeEvidence";
import { VisNetworkCanvas, type GraphCanvasHandle } from "./VisNetworkCanvas";
import {
  graphNeighborhood,
  MAX_NEIGHBORHOOD_DEPTH,
  MIN_NEIGHBORHOOD_DEPTH,
  type GraphEdgeDirection
} from "./neighborhood";
import {
  graphRenderingProfile,
  visibleGraphEdges,
  type GraphLayoutStyle
} from "./renderingProfile";
import {
  graphReducer,
  initialGraphStateForModel,
  type GraphChangeType
} from "./state";

const CHANGE_TYPES: Array<{
  value: GraphChangeType;
  label: string;
}> = [
  { value: "added", label: "Added" },
  { value: "removed", label: "Removed" },
  { value: "changed", label: "Changed" },
  { value: "unchanged", label: "Context" }
];

const FIXED_LAYOUT_STATUS: Record<Exclude<GraphLayoutStyle, "automatic">, string> = {
  hierarchical: "Depth-layer layout",
  circle: "Circle layout",
  concentric: "Concentric layout",
  spiral: "Spiral layout",
  grid: "Square grid layout"
};

const EDGE_DIRECTIONS: readonly GraphEdgeDirection[] = ["both", "outgoing", "incoming"];

function isEditableKeyboardTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement
    && (target.isContentEditable
      || target.tagName === "INPUT"
      || target.tagName === "TEXTAREA"
      || target.tagName === "SELECT");
}

export type GraphHost = {
  openSource(source: SourceLocation, revision?: string): void;
  openCommunity?(communityId: number): void;
  queryNode?(operation: "callers" | "callees" | "impact", symbol: string): void;
};

export type CommunityGraphDetail = {
  communityId: number;
  model: GraphViewModel;
  bounded?: {
    limit: number;
    parentMembers: number;
    currentMembers: number;
  } | undefined;
};

export type CompassGraphProps = {
  model: GraphViewModel;
  host: GraphHost;
  communityDetail?: CommunityGraphDetail | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  sourceRevisions?: GraphSourceRevisions | undefined;
  queryResult?: CodeQueryResponse | undefined;
  initialInspectorLayout?: Partial<InspectorLayout> | undefined;
  onInspectorLayoutChange?: ((layout: InspectorLayout) => void) | undefined;
  preferredLayout?: GraphLayoutStyle | undefined;
  toolbarLeading?: ReactNode;
  toolbarLeadingPanel?: ReactNode;
  toolbarLeadingOpen?: boolean | undefined;
  onToolbarLeadingClose?: (() => void) | undefined;
  stageOverlay?: ReactNode;
};

export function CompassGraph({
  model,
  host,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview,
  sourceRevisions,
  queryResult,
  initialInspectorLayout,
  onInspectorLayoutChange,
  preferredLayout = "automatic",
  toolbarLeading,
  toolbarLeadingPanel,
  toolbarLeadingOpen,
  onToolbarLeadingClose,
  stageOverlay
}: CompassGraphProps) {
  const [inspectorLayout, setInspectorLayout] = useState(
    () => normalizeInspectorLayout(initialInspectorLayout)
  );
  const updateInspectorLayout = useCallback((next: InspectorLayout) => {
    const normalized = normalizeInspectorLayout(next);
    setInspectorLayout(normalized);
    onInspectorLayoutChange?.(normalized);
  }, [onInspectorLayoutChange]);
  const activeModel = communityDetail?.model ?? model;
  const viewKey = communityDetail ? `community-${communityDetail.communityId}` : "overview";
  return (
    <CompassGraphView
      key={viewKey}
      model={activeModel}
      host={host}
      detailCommunityId={communityDetail?.communityId}
      communityLoading={communityLoading}
      communityError={communityError}
      onBackToOverview={communityDetail ? onBackToOverview : undefined}
      bounded={communityDetail?.bounded}
      sourceRevisions={sourceRevisions}
      queryResult={queryResult}
      inspectorLayout={inspectorLayout}
      onInspectorLayoutChange={updateInspectorLayout}
      preferredLayout={preferredLayout}
      toolbarLeading={toolbarLeading}
      toolbarLeadingPanel={toolbarLeadingPanel}
      toolbarLeadingOpen={toolbarLeadingOpen}
      onToolbarLeadingClose={onToolbarLeadingClose}
      stageOverlay={stageOverlay}
    />
  );
}

function CompassGraphView({
  model,
  host,
  detailCommunityId,
  communityLoading,
  communityError,
  onBackToOverview,
  bounded,
  sourceRevisions,
  queryResult,
  inspectorLayout,
  onInspectorLayoutChange,
  preferredLayout,
  toolbarLeading,
  toolbarLeadingPanel,
  toolbarLeadingOpen,
  onToolbarLeadingClose,
  stageOverlay
}: {
  model: GraphViewModel;
  host: GraphHost;
  detailCommunityId?: number | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  bounded?: CommunityGraphDetail["bounded"];
  sourceRevisions?: GraphSourceRevisions | undefined;
  queryResult?: CodeQueryResponse | undefined;
  inspectorLayout: InspectorLayout;
  onInspectorLayoutChange(layout: InspectorLayout): void;
  preferredLayout: GraphLayoutStyle;
  toolbarLeading?: ReactNode;
  toolbarLeadingPanel?: ReactNode;
  toolbarLeadingOpen?: boolean | undefined;
  onToolbarLeadingClose?: (() => void) | undefined;
  stageOverlay?: ReactNode;
}) {
  const [state, dispatch] = useReducer(
    graphReducer,
    model,
    (initial) => initialGraphStateForModel(initial, preferredLayout)
  );
  const [hover, setHover] = useState<GraphHover | null>(null);
  const [edgeHover, setEdgeHover] = useState<GraphEdgeHover | null>(null);
  const canvasRef = useRef<GraphCanvasHandle>(null);
  const hostRef = useRef(host);
  hostRef.current = host;
  const nodeById = useMemo(
    () => new Map(model.nodes.map((node) => [node.id, node])),
    [model.nodes]
  );
  const edgeById = useMemo(
    () => new Map(model.edges.map((edge) => [edge.id, edge])),
    [model.edges]
  );
  const graphIndex = useMemo(() => {
    const neighborIds = new Map<string, Set<string>>();
    const edges = new Map<string, GraphViewModel["edges"]>();
    for (const edge of model.edges) {
      const sourceNeighbors = neighborIds.get(edge.source) ?? new Set<string>();
      sourceNeighbors.add(edge.target);
      neighborIds.set(edge.source, sourceNeighbors);
      const targetNeighbors = neighborIds.get(edge.target) ?? new Set<string>();
      targetNeighbors.add(edge.source);
      neighborIds.set(edge.target, targetNeighbors);
      const sourceEdges = edges.get(edge.source) ?? [];
      sourceEdges.push(edge);
      edges.set(edge.source, sourceEdges);
      const targetEdges = edges.get(edge.target) ?? [];
      targetEdges.push(edge);
      edges.set(edge.target, targetEdges);
    }
    return { neighborIds, edges };
  }, [model.edges]);
  const searchIndex = useMemo(() => model.nodes.map((node) => ({
    node,
    text: [node.label, node.source?.file, node.kind]
      .filter((value) => value !== undefined)
      .join("\n")
      .toLocaleLowerCase()
  })), [model.nodes]);
  const renderedEdgeCount = useMemo(
    () => visibleGraphEdges(model).length,
    [model]
  );
  const selected = state.focusedNodeId
    ? nodeById.get(state.focusedNodeId)
    : undefined;
  const selectedNeighborhood = useMemo(
    () => selected
      ? graphNeighborhood(
        model,
        selected.id,
        state.neighborhoodDepth,
        state.edgeDirection
      )
      : null,
    [
      model,
      selected,
      state.edgeDirection,
      state.neighborhoodDepth
    ]
  );
  const hovered = hover ? nodeById.get(hover.nodeId) : undefined;
  const hoveredActivation = hovered
    ? graphNodeActivation(model, hovered, detailCommunityId)
    : undefined;
  const hoveredEdge = edgeHover ? edgeById.get(edgeHover.edgeId) : undefined;
  const hoveredEdgeSource = hoveredEdge ? nodeById.get(hoveredEdge.source) : undefined;
  const hoveredEdgeTarget = hoveredEdge ? nodeById.get(hoveredEdge.target) : undefined;
  const comparisonMode = useMemo(
    () => model.nodes.some((node) => node.change !== undefined)
      || model.edges.some((edge) => edge.change !== undefined),
    [model.edges, model.nodes]
  );
  const changeCounts = useMemo(() => {
    const counts = new Map<GraphChangeType, number>();
    for (const node of model.nodes) {
      const change = node.change ?? "unchanged";
      counts.set(change, (counts.get(change) ?? 0) + 1);
    }
    return counts;
  }, [model.nodes]);
  const neighbors = useMemo(() => {
    if (!selected) return [];
    return [...(graphIndex.neighborIds.get(selected.id) ?? [])]
      .map((id) => nodeById.get(id))
      .filter((node) => node !== undefined)
      .sort((left, right) => left.label.localeCompare(right.label));
  }, [graphIndex.neighborIds, nodeById, selected]);
  const connectedEdges = useMemo(
    () => selected
      ? graphIndex.edges.get(selected.id) ?? []
      : [],
    [graphIndex.edges, selected]
  );
  const deferredQuery = useDeferredValue(state.query);
  const matches = useMemo(() => {
    const query = deferredQuery.trim().toLocaleLowerCase();
    if (!query) return [];
    const found = [];
    for (const entry of searchIndex) {
      if (entry.text.includes(query)) found.push(entry.node);
      if (found.length === 20) break;
    }
    return found;
  }, [deferredQuery, searchIndex]);

  const focus = useCallback((nodeId: string) => {
    setHover(null);
    setEdgeHover(null);
    dispatch({ type: "focus", nodeId });
  }, []);
  const clear = useCallback(() => {
    setHover(null);
    setEdgeHover(null);
    dispatch({ type: "clearFocus" });
  }, []);
  useEffect(() => {
    if (state.focusedNodeId && !nodeById.has(state.focusedNodeId)) clear();
  }, [clear, nodeById, state.focusedNodeId]);
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        event.metaKey
        || event.ctrlKey
        || event.altKey
        || isEditableKeyboardTarget(event.target)
      ) return;
      const key = event.key.toLocaleLowerCase();
      let handled = true;
      if (key === "f") {
        if (event.shiftKey && selectedNeighborhood) {
          canvasRef.current?.fitSelection([...selectedNeighborhood.nodeIds]);
        } else if (!event.shiftKey) {
          canvasRef.current?.fit();
        } else {
          handled = false;
        }
      } else if (key === "+" || key === "=") {
        canvasRef.current?.zoomIn();
      } else if (key === "-" || key === "_") {
        canvasRef.current?.zoomOut();
      } else if (key === "0") {
        canvasRef.current?.resetZoom();
      } else if (key === "i" && selected) {
        dispatch({ type: "setIsolation", isolated: !state.isolateSelection });
      } else if (key === "[") {
        dispatch({
          type: "setNeighborhoodDepth",
          depth: Math.max(MIN_NEIGHBORHOOD_DEPTH, state.neighborhoodDepth - 1)
        });
      } else if (key === "]") {
        dispatch({
          type: "setNeighborhoodDepth",
          depth: Math.min(MAX_NEIGHBORHOOD_DEPTH, state.neighborhoodDepth + 1)
        });
      } else if (key === "d") {
        const index = EDGE_DIRECTIONS.indexOf(state.edgeDirection);
        dispatch({
          type: "setEdgeDirection",
          direction: EDGE_DIRECTIONS[(index + 1) % EDGE_DIRECTIONS.length] ?? "both"
        });
      } else if (key === "m") {
        dispatch({ type: "setMinimap", visible: !state.showMinimap });
      } else {
        handled = false;
      }
      if (handled) event.preventDefault();
    };
    document.addEventListener("keydown", handleKeyDown, { capture: true });
    return () => document.removeEventListener("keydown", handleKeyDown, { capture: true });
  }, [
    selected,
    selectedNeighborhood,
    state.edgeDirection,
    state.isolateSelection,
    state.neighborhoodDepth,
    state.showMinimap
  ]);
  const handleStabilized = useCallback(() => {
    dispatch({ type: "stabilized" });
  }, []);
  const revealCurrentLayout = useCallback(() => {
    dispatch({ type: "revealLayout" });
  }, []);
  const activateNode = useCallback((nodeId: string) => {
    const node = nodeById.get(nodeId);
    if (!node) return;
    const activation = graphNodeActivation(model, node, detailCommunityId);
    if (activation.type === "community") {
      hostRef.current.openCommunity?.(activation.communityId);
    }
    if (activation.type === "source") {
      hostRef.current.openSource(
        activation.source,
        node.change === "removed" ? sourceRevisions?.before : sourceRevisions?.after
      );
    }
  }, [
    detailCommunityId,
    nodeById,
    model.stats.aggregated,
    sourceRevisions?.after,
    sourceRevisions?.before
  ]);
  const activateRelationship = useCallback((edgeId: string) => {
    const edge = edgeById.get(edgeId);
    if (!edge) return;
    const source = navigableRelationshipSource(edge);
    if (!source) return;
    hostRef.current.openSource(
      source,
      edge.change === "removed" ? sourceRevisions?.before : sourceRevisions?.after
    );
  }, [edgeById, sourceRevisions?.after, sourceRevisions?.before]);
  const status = selected && state.isolateSelection && selectedNeighborhood
    ? `Isolated ${selectedNeighborhood.nodeIds.size} nodes · ${state.neighborhoodDepth} hop${state.neighborhoodDepth === 1 ? "" : "s"}`
    : selected
    ? `Inspecting ${selected.label}`
    : state.layoutStyle !== "automatic"
      ? FIXED_LAYOUT_STATUS[state.layoutStyle]
      : state.physicsRunning ? "Layout running" : "Layout paused";
  const loadingCommunity = communityLoading !== undefined && communityLoading !== null
    ? model.communities.find((community) => community.id === communityLoading)
    : undefined;
  const transition = communityLoading !== undefined && communityLoading !== null
    ? {
      kind: "community" as const,
      communityLabel: loadingCommunity?.label ?? `Community ${communityLoading}`
    }
    : state.initialLayoutPending
      ? { kind: "layout" as const }
      : null;

  return (
    <div
      className="compass-workspace"
      style={{
        "--compass-inspector-width": `${inspectorLayout.width}px`
      } as CSSProperties}
    >
      <div
        className="compass-workspace-content"
        data-inspector-collapsed={inspectorLayout.collapsed}
        inert={transition ? true : undefined}
        aria-hidden={transition ? true : undefined}
      >
        <main
          className="compass-graph-stage"
          data-comparison={comparisonMode ? "true" : "false"}
        >
          <VisNetworkCanvas
            ref={canvasRef}
            model={model}
            focusedNodeId={selected?.id ?? null}
            physicsRunning={state.physicsRunning}
            layoutStyle={state.layoutStyle}
            forceLabels={state.forceLabels}
            showEdgeLabels={state.showEdgeLabels}
            isolatedNodeIds={state.isolateSelection
              ? selectedNeighborhood?.nodeIds
              : undefined}
            isolatedEdgeIds={state.isolateSelection
              ? selectedNeighborhood?.edgeIds
              : undefined}
            layoutSpacing={state.layoutSpacing}
            showMinimap={state.showMinimap}
            hiddenCommunities={state.hiddenCommunities}
            hiddenChanges={state.hiddenChanges}
            onFocus={focus}
            onOpenSource={activateNode}
            onOpenRelationshipSource={activateRelationship}
            onHover={setHover}
            onHoverEdge={setEdgeHover}
            onClear={clear}
            onStabilized={handleStabilized}
          />
          {stageOverlay}
          <GraphToolbar
            status={status}
            physicsRunning={state.physicsRunning}
            layoutStyle={state.layoutStyle}
            forceLabels={state.forceLabels}
            showEdgeLabels={state.showEdgeLabels}
            hasSelection={selected !== undefined}
            isolateSelection={state.isolateSelection}
            neighborhoodDepth={state.neighborhoodDepth}
            edgeDirection={state.edgeDirection}
            layoutSpacing={state.layoutSpacing}
            showMinimap={state.showMinimap}
            leadingControls={toolbarLeading}
            leadingPanel={toolbarLeadingPanel}
            leadingPanelOpen={toolbarLeadingOpen}
            onLeadingPanelClose={onToolbarLeadingClose}
            onTogglePhysics={() => dispatch({
              type: "setPhysics",
              running: !state.physicsRunning
            })}
            onLayoutChange={(layout: GraphLayoutStyle) => dispatch({
              type: "setLayout",
              layout,
              runPhysics: layout === "automatic"
                && graphRenderingProfile(model) === "interactive"
            })}
            onZoomOut={() => canvasRef.current?.zoomOut()}
            onResetZoom={() => canvasRef.current?.resetZoom()}
            onZoomIn={() => canvasRef.current?.zoomIn()}
            onFit={() => canvasRef.current?.fit()}
            onFitSelection={() => {
              if (selectedNeighborhood) {
                canvasRef.current?.fitSelection([...selectedNeighborhood.nodeIds]);
              }
            }}
            onReset={() => {
              clear();
              dispatch({ type: "setIsolation", isolated: false });
              canvasRef.current?.reset();
            }}
            onToggleLabels={() => dispatch({
              type: "setLabels",
              visible: !state.forceLabels
            })}
            onToggleEdgeLabels={() => dispatch({
              type: "setEdgeLabels",
              visible: !state.showEdgeLabels
            })}
            onToggleIsolation={() => {
              const isolated = !state.isolateSelection;
              dispatch({ type: "setIsolation", isolated });
              if (isolated && selectedNeighborhood) {
                window.requestAnimationFrame(() => {
                  canvasRef.current?.fitSelection([...selectedNeighborhood.nodeIds]);
                });
              }
            }}
            onNeighborhoodDepthChange={(depth) => dispatch({
              type: "setNeighborhoodDepth",
              depth
            })}
            onEdgeDirectionChange={(direction) => dispatch({
              type: "setEdgeDirection",
              direction
            })}
            onLayoutSpacingChange={(spacing) => dispatch({
              type: "setLayoutSpacing",
              spacing
            })}
            onToggleMinimap={() => dispatch({
              type: "setMinimap",
              visible: !state.showMinimap
            })}
            onBack={onBackToOverview}
          />
          {comparisonMode && (
            <div className="compass-change-legend" aria-label="Graph change filters">
              {CHANGE_TYPES
                .filter(({ value }) => (changeCounts.get(value) ?? 0) > 0)
                .map(({ value, label }) => {
                  const visible = !state.hiddenChanges.has(value);
                  return (
                    <button
                      key={value}
                      type="button"
                      data-change={value}
                      aria-pressed={visible}
                      onClick={() => dispatch({ type: "toggleChange", change: value })}
                    >
                      <span aria-hidden="true" />
                      {label}
                      <small>{changeCounts.get(value) ?? 0}</small>
                    </button>
                  );
                })}
            </div>
          )}
          {communityError && (
            <div
              className="absolute bottom-4 left-4 z-20 max-w-md rounded-md border border-destructive/50 bg-background/95 px-3 py-2 text-sm text-destructive shadow-lg"
              role="alert"
            >
              {communityError}
            </div>
          )}
          {bounded && (
            <div className="compass-bounded-notice" role="status">
              <strong>Partial community comparison</strong>
              <span>
                This view is limited to {bounded.limit.toLocaleString()} nodes. Increase{" "}
                <code>compass.graphNodeLimit</code> to inspect all{" "}
                {Math.max(bounded.parentMembers, bounded.currentMembers).toLocaleString()} symbols.
              </span>
            </div>
          )}
          {hover && hovered && hoveredActivation && (
            <NodeHoverCard
              node={hovered}
              hover={hover}
              activation={hoveredActivation}
            />
          )}
          {edgeHover && hoveredEdge && hoveredEdgeSource && hoveredEdgeTarget && (
            <EdgeHoverCard
              edge={hoveredEdge}
              sourceNode={hoveredEdgeSource}
              targetNode={hoveredEdgeTarget}
              hover={edgeHover}
            />
          )}
        </main>
        {!inspectorLayout.collapsed && (
          <InspectorResizeHandle
            width={inspectorLayout.width}
            onResize={(width) => onInspectorLayoutChange({
              ...inspectorLayout,
              width
            })}
          />
        )}
        <GraphInspector
          model={model}
          selected={selected}
          neighbors={neighbors}
          connectedEdges={connectedEdges}
          query={state.query}
          matches={matches}
          hiddenCommunities={state.hiddenCommunities}
          comparisonMode={comparisonMode}
          sourceRevisions={sourceRevisions}
          queryResult={queryResult}
          renderedEdgeCount={renderedEdgeCount}
          onQueryChange={(query) => dispatch({ type: "search", query })}
          onFocus={focus}
          onOpenSource={host.openSource}
          onOpenCommunity={detailCommunityId === undefined ? host.openCommunity : undefined}
          onQueryNode={host.queryNode}
          onToggleCommunity={(communityId) => dispatch({
            type: "toggleCommunity",
            communityId
          })}
          onSetAllVisible={(visible) => dispatch({
            type: "setHiddenCommunities",
            communityIds: visible ? [] : model.communities.map((community) => community.id)
          })}
          collapsed={inspectorLayout.collapsed}
          onToggleCollapsed={() => onInspectorLayoutChange({
            ...inspectorLayout,
            collapsed: !inspectorLayout.collapsed
          })}
        />
      </div>
      {transition?.kind === "community" ? (
        <GraphTransitionScreen
          kind="community"
          communityLabel={transition.communityLabel}
        />
      ) : transition?.kind === "layout" ? (
        <GraphTransitionScreen kind="layout" onShowGraph={revealCurrentLayout} />
      ) : null}
    </div>
  );
}
