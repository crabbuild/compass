import type {
  GraphViewModel,
  HistoryChangeCounts,
  HistoryTimeline,
  SourceLocation
} from "@compass/viewer";

export type HistoryOperation =
  | "Load history"
  | "Load more history"
  | "Enable history"
  | "Load graph"
  | "Build graph"
  | "Compare revisions"
  | "Compare community"
  | "Load change counts"
  | "Open community"
  | "Open source"
  | "Query revision";

export type HistoryWebviewMessage =
  | { type: "ready" }
  | { type: "retryTimeline" }
  | { type: "loadMoreTimeline" }
  | { type: "enableHistory" }
  | { type: "loadRevision"; commit: string }
  | { type: "buildRevision"; commit: string }
  | { type: "compare"; commit: string; parent: string }
  | {
    type: "compareCommunity";
    requestId: string;
    commit: string;
    parent: string;
    currentIdentity: { realization: string; fingerprint: string };
    parentIdentity: { realization: string; fingerprint: string };
    communityId: number;
    hasCurrent: boolean;
    hasParent: boolean;
  }
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
  | { type: "timeline"; timeline: HistoryTimeline; repositoryId: string; generation: number }
  | { type: "timelinePage"; timeline: HistoryTimeline; repositoryId: string; generation: number }
  | { type: "timelinePageError"; message: string; generation: number }
  | { type: "bootstrapError"; message: string }
  | { type: "enableRunning" }
  | { type: "enableSucceeded" }
  | { type: "enableCancelled" }
  | { type: "enableFailed"; message: string }
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
    parentRealization: string;
    parentFingerprint: string;
    currentGraph: GraphViewModel;
    parentGraph: GraphViewModel;
    semanticDiff: unknown;
    counts?: HistoryChangeCounts;
  }
  | {
    type: "communityComparison";
    requestId: string;
    commit: string;
    parent: string;
    communityId: number;
    currentGraph?: GraphViewModel;
    parentGraph?: GraphViewModel;
    nodeLimit: number;
  }
  | {
    type: "communityComparisonError";
    requestId: string;
    commit: string;
    parent: string;
    communityId: number;
    message: string;
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
    case "retryTimeline": return "Load history";
    case "loadMoreTimeline": return "Load more history";
    case "enableHistory": return "Enable history";
    case "loadRevision": return "Load graph";
    case "buildRevision": return "Build graph";
    case "compare": return "Compare revisions";
    case "compareCommunity": return "Compare community";
    case "changeCounts": return "Load change counts";
    case "openCommunity": return "Open community";
    case "openSource": return "Open source";
    case "queryRevision": return "Query revision";
    default: return "Load graph";
  }
}
