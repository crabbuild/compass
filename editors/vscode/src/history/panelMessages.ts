import type {
  GraphViewModel,
  HistoryChangeCounts,
  HistoryTimeline,
  SourceLocation
} from "@compass/viewer";

export type HistoryOperation =
  | "Load graph"
  | "Build graph"
  | "Compare revisions"
  | "Load change counts"
  | "Open community"
  | "Open source"
  | "Query revision";

export type HistoryWebviewMessage =
  | { type: "ready" }
  | { type: "loadRevision"; commit: string }
  | { type: "buildRevision"; commit: string }
  | { type: "compare"; commit: string; parent: string }
  | { type: "queryRevision"; commit: string }
  | { type: "changeCounts"; commit: string }
  | {
    type: "openCommunity";
    requestId: string;
    commit: string;
    realization: string;
    fingerprint: string;
    communityId: number;
  }
  | {
    type: "openSource";
    commit: string;
    repositoryId: string;
    source: SourceLocation;
  };

export type HistoryHostMessage =
  | { type: "timeline"; timeline: HistoryTimeline; repositoryId: string }
  | {
    type: "graph";
    commit: string;
    realization: string;
    fingerprint: string;
    graph: GraphViewModel;
  }
  | {
    type: "communityGraph";
    requestId: string;
    commit: string;
    communityId: number;
    graph: GraphViewModel;
  }
  | {
    type: "communityError";
    requestId: string;
    commit: string;
    communityId: number;
    message: string;
  }
  | {
    type: "comparison";
    commit: string;
    parent: string;
    realization: string;
    fingerprint: string;
    currentGraph: GraphViewModel;
    parentGraph: GraphViewModel;
    semanticDiff: unknown;
  }
  | { type: "changeCounts"; commit: string; counts: HistoryChangeCounts }
  | { type: "buildRunning"; commit: string }
  | { type: "buildSucceeded"; commit: string }
  | { type: "buildFailed"; commit: string; message: string }
  | { type: "buildCancelled"; commit: string }
  | {
    type: "error";
    operation: HistoryOperation;
    commit?: string;
    message: string;
  };

export function historyOperationFor(message: unknown): HistoryOperation {
  const type = typeof message === "object" && message !== null && "type" in message
    ? (message as { type?: unknown }).type
    : undefined;
  switch (type) {
    case "loadRevision": return "Load graph";
    case "buildRevision": return "Build graph";
    case "compare": return "Compare revisions";
    case "changeCounts": return "Load change counts";
    case "openCommunity": return "Open community";
    case "openSource": return "Open source";
    case "queryRevision": return "Query revision";
    default: return "Load graph";
  }
}
