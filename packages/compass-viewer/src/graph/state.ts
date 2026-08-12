import type { GraphNode, GraphViewModel } from "../contracts/graph";
import {
  graphRenderingProfile,
  type GraphLayoutStyle
} from "./renderingProfile";

export type GraphChangeType = NonNullable<GraphNode["change"]>;

export type GraphState = {
  focusedNodeId: string | null;
  physicsRunning: boolean;
  layoutStyle: GraphLayoutStyle;
  initialLayoutPending: boolean;
  forceLabels: boolean;
  showEdgeLabels: boolean;
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
  | { type: "toggleCommunity"; communityId: number }
  | { type: "setHiddenCommunities"; communityIds: number[] }
  | { type: "toggleChange"; change: GraphChangeType }
  | { type: "search"; query: string };

export const initialGraphState: GraphState = {
  focusedNodeId: null,
  physicsRunning: true,
  layoutStyle: "automatic",
  initialLayoutPending: true,
  forceLabels: false,
  showEdgeLabels: false,
  hiddenCommunities: new Set<number>(),
  hiddenChanges: new Set<GraphChangeType>(),
  query: ""
};

export function initialGraphStateForModel(
  model: GraphViewModel,
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
  if (graphRenderingProfile(model) === "interactive") return initialGraphState;
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
      return { ...state, focusedNodeId: null };
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
