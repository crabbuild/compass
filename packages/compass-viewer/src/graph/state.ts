export type GraphState = {
  focusedNodeId: string | null;
  physicsRunning: boolean;
  forceLabels: boolean;
  hiddenCommunities: ReadonlySet<number>;
  query: string;
};

export type GraphAction =
  | { type: "focus"; nodeId: string }
  | { type: "clearFocus" }
  | { type: "stabilized" }
  | { type: "setPhysics"; running: boolean }
  | { type: "setLabels"; visible: boolean }
  | { type: "toggleCommunity"; communityId: number }
  | { type: "search"; query: string };

export const initialGraphState: GraphState = {
  focusedNodeId: null,
  physicsRunning: true,
  forceLabels: false,
  hiddenCommunities: new Set<number>(),
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
      return { ...state, physicsRunning: false };
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
  }
}
