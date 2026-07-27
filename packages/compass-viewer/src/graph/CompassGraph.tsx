import {
  useCallback,
  useMemo,
  useReducer,
  useRef,
  useState,
  type CSSProperties
} from "react";
import type { GraphViewModel, SourceLocation } from "../contracts/graph";
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
import type { GraphSourceRevisions } from "./ChangeEvidence";
import { VisNetworkCanvas, type GraphCanvasHandle } from "./VisNetworkCanvas";
import {
  graphReducer,
  initialGraphState,
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

export type GraphHost = {
  openSource(source: SourceLocation, revision?: string): void;
  openCommunity?(communityId: number): void;
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
  initialInspectorLayout?: Partial<InspectorLayout> | undefined;
  onInspectorLayoutChange?: ((layout: InspectorLayout) => void) | undefined;
};

export function CompassGraph({
  model,
  host,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview,
  sourceRevisions,
  initialInspectorLayout,
  onInspectorLayoutChange
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
      inspectorLayout={inspectorLayout}
      onInspectorLayoutChange={updateInspectorLayout}
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
  inspectorLayout,
  onInspectorLayoutChange
}: {
  model: GraphViewModel;
  host: GraphHost;
  detailCommunityId?: number | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
  bounded?: CommunityGraphDetail["bounded"];
  sourceRevisions?: GraphSourceRevisions | undefined;
  inspectorLayout: InspectorLayout;
  onInspectorLayoutChange(layout: InspectorLayout): void;
}) {
  const [state, dispatch] = useReducer(graphReducer, initialGraphState);
  const [hover, setHover] = useState<GraphHover | null>(null);
  const canvasRef = useRef<GraphCanvasHandle>(null);
  const hostRef = useRef(host);
  hostRef.current = host;
  const selected = model.nodes.find((node) => node.id === state.focusedNodeId);
  const hovered = hover ? model.nodes.find((node) => node.id === hover.nodeId) : undefined;
  const hoveredActivation = hovered
    ? graphNodeActivation(model, hovered, detailCommunityId)
    : undefined;
  const comparisonMode = model.nodes.some((node) => node.change !== undefined)
    || model.edges.some((edge) => edge.change !== undefined);
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
    const ids = new Set<string>();
    for (const edge of model.edges) {
      if (edge.source === selected.id) ids.add(edge.target);
      if (edge.target === selected.id) ids.add(edge.source);
    }
    return [...ids]
      .map((id) => model.nodes.find((node) => node.id === id))
      .filter((node) => node !== undefined)
      .sort((left, right) => left.label.localeCompare(right.label));
  }, [model.edges, model.nodes, selected]);
  const matches = useMemo(() => {
    const query = state.query.trim().toLocaleLowerCase();
    if (!query) return [];
    return model.nodes
      .filter((node) => node.label.toLocaleLowerCase().includes(query)
        || node.source?.file.toLocaleLowerCase().includes(query)
        || node.kind?.toLocaleLowerCase().includes(query))
      .slice(0, 20);
  }, [model.nodes, state.query]);

  const focus = useCallback((nodeId: string) => {
    setHover(null);
    dispatch({ type: "focus", nodeId });
  }, []);
  const clear = useCallback(() => {
    setHover(null);
    dispatch({ type: "clearFocus" });
  }, []);
  const handleStabilized = useCallback(() => {
    dispatch({ type: "stabilized" });
  }, []);
  const revealCurrentLayout = useCallback(() => {
    dispatch({ type: "revealLayout" });
  }, []);
  const activateNode = useCallback((nodeId: string) => {
    const node = model.nodes.find((candidate) => candidate.id === nodeId);
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
    model.nodes,
    model.stats.aggregated,
    sourceRevisions?.after,
    sourceRevisions?.before
  ]);
  const status = selected
    ? `Inspecting ${selected.label}`
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
            forceLabels={state.forceLabels}
            hiddenCommunities={state.hiddenCommunities}
            hiddenChanges={state.hiddenChanges}
            onFocus={focus}
            onOpenSource={activateNode}
            onHover={setHover}
            onClear={clear}
            onStabilized={handleStabilized}
          />
          <GraphToolbar
            status={status}
            physicsRunning={state.physicsRunning}
            forceLabels={state.forceLabels}
            onTogglePhysics={() => dispatch({
              type: "setPhysics",
              running: !state.physicsRunning
            })}
            onFit={() => canvasRef.current?.fit()}
            onReset={() => {
              clear();
              canvasRef.current?.reset();
            }}
            onToggleLabels={() => dispatch({
              type: "setLabels",
              visible: !state.forceLabels
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
          connectedEdges={selected
            ? model.edges.filter((edge) => edge.source === selected.id || edge.target === selected.id)
            : []}
          query={state.query}
          matches={matches}
          hiddenCommunities={state.hiddenCommunities}
          comparisonMode={comparisonMode}
          sourceRevisions={sourceRevisions}
          onQueryChange={(query) => dispatch({ type: "search", query })}
          onFocus={focus}
          onOpenSource={host.openSource}
          onOpenCommunity={detailCommunityId === undefined ? host.openCommunity : undefined}
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
