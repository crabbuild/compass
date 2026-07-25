import { useCallback, useMemo, useReducer, useRef, useState } from "react";
import type { GraphViewModel, SourceLocation } from "../contracts/graph";
import { GraphInspector } from "./GraphInspector";
import { GraphToolbar } from "./GraphToolbar";
import { graphNodeActivation } from "./nodeActivation";
import { NodeHoverCard, type GraphHover } from "./NodeHoverCard";
import { VisNetworkCanvas, type GraphCanvasHandle } from "./VisNetworkCanvas";
import { graphReducer, initialGraphState } from "./state";

export type GraphHost = {
  openSource(source: SourceLocation): void;
  openCommunity?(communityId: number): void;
};

export function CompassGraph({
  model,
  host,
  communityDetail,
  communityLoading,
  communityError,
  onBackToOverview
}: {
  model: GraphViewModel;
  host: GraphHost;
  communityDetail?: { communityId: number; model: GraphViewModel } | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
}) {
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
    />
  );
}

function CompassGraphView({
  model,
  host,
  detailCommunityId,
  communityLoading,
  communityError,
  onBackToOverview
}: {
  model: GraphViewModel;
  host: GraphHost;
  detailCommunityId?: number | undefined;
  communityLoading?: number | null | undefined;
  communityError?: string | undefined;
  onBackToOverview?: (() => void) | undefined;
}) {
  const [state, dispatch] = useReducer(graphReducer, initialGraphState);
  const [hover, setHover] = useState<GraphHover | null>(null);
  const canvasRef = useRef<GraphCanvasHandle>(null);
  const hostRef = useRef(host);
  hostRef.current = host;
  const selected = model.nodes.find((node) => node.id === state.focusedNodeId);
  const hovered = hover ? model.nodes.find((node) => node.id === hover.nodeId) : undefined;
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
  const activateNode = useCallback((nodeId: string) => {
    const node = model.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    const activation = graphNodeActivation(model, node, detailCommunityId);
    if (activation.type === "community") {
      hostRef.current.openCommunity?.(activation.communityId);
    }
    if (activation.type === "source") hostRef.current.openSource(activation.source);
  }, [detailCommunityId, model.nodes, model.stats.aggregated]);
  const status = communityLoading !== undefined && communityLoading !== null
    ? `Loading community ${communityLoading}`
    : selected
    ? `Inspecting ${selected.label}`
    : state.physicsRunning ? "Layout settling" : "Layout paused";

  return (
    <div className="compass-workspace">
      <main className="compass-graph-stage">
        <VisNetworkCanvas
          ref={canvasRef}
          model={model}
          focusedNodeId={state.focusedNodeId}
          physicsRunning={state.physicsRunning}
          forceLabels={state.forceLabels}
          hiddenCommunities={state.hiddenCommunities}
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
        {communityError && (
          <div
            className="absolute bottom-4 left-4 z-20 max-w-md rounded-md border border-destructive/50 bg-background/95 px-3 py-2 text-sm text-destructive shadow-lg"
            role="alert"
          >
            {communityError}
          </div>
        )}
        {hover && hovered && <NodeHoverCard node={hovered} hover={hover} />}
      </main>
      <GraphInspector
        model={model}
        selected={selected}
        neighbors={neighbors}
        query={state.query}
        matches={matches}
        hiddenCommunities={state.hiddenCommunities}
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
      />
    </div>
  );
}
