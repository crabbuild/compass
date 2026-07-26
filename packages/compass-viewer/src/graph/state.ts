import type { GraphNode } from "../contracts/graph";

export type GraphChangeType = NonNullable<GraphNode["change"]>;

export type GraphState = {
  focusedNodeId: string | null;
  physicsRunning: boolean;
  forceLabels: boolean;
  hiddenCommunities: ReadonlySet<number>;
  hiddenChanges: ReadonlySet<GraphChangeType>;
  query: string;
};

export type GraphAction =
  | { type: "focus"; nodeId: string }
  | { type: "clearFocus" }
  | { type: "stabilized" }
  | { type: "setPhysics"; running: boolean }
  | { type: "setLabels"; visible: boolean }
  | { type: "toggleCommunity"; communityId: number }
  | { type: "setHiddenCommunities"; communityIds: number[] }
  | { type: "toggleChange"; change: GraphChangeType }
  | { type: "search"; query: string };

export const initialGraphState: GraphState = {
  focusedNodeId: null,
  physicsRunning: true,
  forceLabels: false,
  hiddenCommunities: new Set<number>(),
  hiddenChanges: new Set<GraphChangeType>(),
  query: ""
};

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
      return state.physicsRunning ? { ...state, physicsRunning: false } : state;
    case "setPhysics":
      return { ...state, physicsRunning: action.running };
    case "setLabels":
      return { ...state, forceLabels: action.visible };
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
