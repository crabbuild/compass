import {
  useCallback,
  useDeferredValue,
  useMemo,
  useReducer,
  useRef,
  useState,
  type CSSProperties
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
  preferredLayout = "automatic"
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
  preferredLayout
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
  const status = selected
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
            focusedNodeId={state.focusedNodeId}
            physicsRunning={state.physicsRunning}
            layoutStyle={state.layoutStyle}
            forceLabels={state.forceLabels}
            showEdgeLabels={state.showEdgeLabels}
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
          <GraphToolbar
            status={status}
            physicsRunning={state.physicsRunning}
            layoutStyle={state.layoutStyle}
            forceLabels={state.forceLabels}
            showEdgeLabels={state.showEdgeLabels}
            hasSelection={selected !== undefined}
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
              if (selected) canvasRef.current?.fitSelection(selected.id);
            }}
            onReset={() => {
              clear();
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
