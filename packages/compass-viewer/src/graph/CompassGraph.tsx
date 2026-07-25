import { useCallback, useMemo, useReducer, useRef, useState } from "react";
import type { GraphViewModel, SourceLocation } from "../contracts/graph";
import { GraphInspector } from "./GraphInspector";
import { GraphToolbar } from "./GraphToolbar";
import { NodeHoverCard, type GraphHover } from "./NodeHoverCard";
import { VisNetworkCanvas, type GraphCanvasHandle } from "./VisNetworkCanvas";
import { navigableSource } from "./sourceNavigation";
import { graphReducer, initialGraphState } from "./state";

export type GraphHost = {
  openSource(source: SourceLocation): void;
};

export function CompassGraph({
  model,
  host
}: {
  model: GraphViewModel;
  host: GraphHost;
}) {
  const [state, dispatch] = useReducer(graphReducer, initialGraphState);
  const [hover, setHover] = useState<GraphHover | null>(null);
  const canvasRef = useRef<GraphCanvasHandle>(null);
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
  const openNodeSource = useCallback((nodeId: string) => {
    const node = model.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    const source = navigableSource(node);
    if (source) host.openSource(source);
  }, [host, model.nodes]);
  const status = selected
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
          onOpenSource={openNodeSource}
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
        />
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
