import type { GraphNode, GraphViewModel } from "../contracts/graph";
import type { GraphLayoutStyle } from "./renderingProfile";
import type { GraphEdgeDirection } from "./neighborhood";

export type GraphChangeType = NonNullable<GraphNode["change"]>;
export type GraphLayoutSpacing = 0.75 | 1 | 1.25 | 1.5;

export type GraphState = {
  focusedNodeId: string | null;
  physicsRunning: boolean;
  layoutStyle: GraphLayoutStyle;
  initialLayoutPending: boolean;
  forceLabels: boolean;
  showEdgeLabels: boolean;
  isolateSelection: boolean;
  neighborhoodDepth: number;
  edgeDirection: GraphEdgeDirection;
  layoutSpacing: GraphLayoutSpacing;
  showMinimap: boolean;
  hiddenCommunities: ReadonlySet<number>;
  hiddenChanges: ReadonlySet<GraphChangeType>;
  query: string;
};

export type GraphAction =
  | { type: "focus"; nodeId: string }
  | { type: "clearFocus" }
  | { type: "stabilized" }
  | { type: "revealLayout" }
  | { type: "setPhysics"; running: boolean }
  | { type: "setLayout"; layout: GraphLayoutStyle; runPhysics: boolean }
  | { type: "setLabels"; visible: boolean }
  | { type: "setEdgeLabels"; visible: boolean }
  | { type: "setIsolation"; isolated: boolean }
  | { type: "setNeighborhoodDepth"; depth: number }
  | { type: "setEdgeDirection"; direction: GraphEdgeDirection }
  | { type: "setLayoutSpacing"; spacing: GraphLayoutSpacing }
  | { type: "setMinimap"; visible: boolean }
  | { type: "toggleCommunity"; communityId: number }
  | { type: "setHiddenCommunities"; communityIds: number[] }
  | { type: "toggleChange"; change: GraphChangeType }
  | { type: "search"; query: string };

export const initialGraphState: GraphState = {
  focusedNodeId: null,
  physicsRunning: false,
  layoutStyle: "automatic",
  initialLayoutPending: false,
  forceLabels: false,
  showEdgeLabels: false,
  isolateSelection: false,
  neighborhoodDepth: 1,
  edgeDirection: "both",
  layoutSpacing: 1,
  showMinimap: true,
  hiddenCommunities: new Set<number>(),
  hiddenChanges: new Set<GraphChangeType>(),
  query: ""
};

export function initialGraphStateForModel(
  _model: GraphViewModel,
  preferredLayout: GraphLayoutStyle = "automatic"
): GraphState {
  if (preferredLayout !== "automatic") {
    return {
      ...initialGraphState,
      layoutStyle: preferredLayout,
      physicsRunning: false,
      initialLayoutPending: false
    };
  }
  return {
    ...initialGraphState,
    physicsRunning: false,
    initialLayoutPending: false
  };
}

export function graphReducer(state: GraphState, action: GraphAction): GraphState {
  switch (action.type) {
    case "focus":
      return {
        ...state,
        focusedNodeId: action.nodeId,
        physicsRunning: false
      };
    case "clearFocus":
      return { ...state, focusedNodeId: null, isolateSelection: false };
    case "stabilized":
      return state.physicsRunning || state.initialLayoutPending
        ? { ...state, physicsRunning: false, initialLayoutPending: false }
        : state;
    case "revealLayout":
      return {
        ...state,
        physicsRunning: false,
        initialLayoutPending: false
      };
    case "setPhysics":
      return { ...state, physicsRunning: action.running };
    case "setLayout":
      return {
        ...state,
        layoutStyle: action.layout,
        physicsRunning: action.runPhysics,
        initialLayoutPending: false
      };
    case "setLabels":
      return { ...state, forceLabels: action.visible };
    case "setEdgeLabels":
      return { ...state, showEdgeLabels: action.visible };
    case "setIsolation":
      return { ...state, isolateSelection: action.isolated };
    case "setNeighborhoodDepth":
      return {
        ...state,
        neighborhoodDepth: Math.max(1, Math.min(4, Math.trunc(action.depth)))
      };
    case "setEdgeDirection":
      return { ...state, edgeDirection: action.direction };
    case "setLayoutSpacing":
      return { ...state, layoutSpacing: action.spacing };
    case "setMinimap":
      return { ...state, showMinimap: action.visible };
    case "search":
      return { ...state, query: action.query };
    case "toggleCommunity": {
      const hiddenCommunities = new Set(state.hiddenCommunities);
      if (hiddenCommunities.has(action.communityId)) {
        hiddenCommunities.delete(action.communityId);
      } else {
        hiddenCommunities.add(action.communityId);
      }
      return { ...state, hiddenCommunities };
    }
    case "setHiddenCommunities":
      return { ...state, hiddenCommunities: new Set(action.communityIds) };
    case "toggleChange": {
      const hiddenChanges = new Set(state.hiddenChanges);
      if (hiddenChanges.has(action.change)) hiddenChanges.delete(action.change);
      else hiddenChanges.add(action.change);
      return { ...state, hiddenChanges };
    }
  }
}
