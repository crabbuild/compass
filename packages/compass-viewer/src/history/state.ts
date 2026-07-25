import type { HistoryTimeline } from "../contracts/history";

export type HistoryState = {
  selected: string;
  query: string;
  selectedParent?: string | undefined;
  loading: boolean;
  building: ReadonlySet<string>;
};
export type HistoryAction =
  | { type: "select"; commit: string }
  | { type: "search"; query: string }
  | { type: "parent"; commit?: string }
  | { type: "loading"; loading: boolean }
  | { type: "building"; commit: string; building: boolean };

export function initialHistoryState(timeline: HistoryTimeline): HistoryState {
  return {
    selected: timeline.selectedHead,
    query: "",
    loading: false,
    building: new Set()
  };
}

export function historyReducer(state: HistoryState, action: HistoryAction): HistoryState {
  switch (action.type) {
    case "select":
      return { ...state, selected: action.commit, selectedParent: undefined };
    case "search":
      return { ...state, query: action.query };
    case "parent":
      return { ...state, selectedParent: action.commit };
    case "loading":
      return { ...state, loading: action.loading };
    case "building": {
      const building = new Set(state.building);
      if (action.building) building.add(action.commit);
      else building.delete(action.commit);
      return { ...state, building };
    }
  }
}
