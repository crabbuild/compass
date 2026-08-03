import { createRoot } from "react-dom/client";
import {
  HistoryTimelineSchema,
  HistoryChangeCountsSchema,
  SemanticDiffReportSchema,
  HistoryWorkspace,
  WorkspaceState,
  GraphViewModelSchema,
  compareGraphs,
  comparisonFromSemanticDiff,
  type GraphViewModel,
  type GraphComparison,
  type CommunityGraphDetail,
  type HistoryBuildState,
  type HistoryChangeCounts,
  type HistoryOperationError,
  type SemanticDiffReport,
  type HistoryTimeline
} from "@compass/viewer";
import type {
  HistoryHostMessage,
  HistoryWebviewMessage
} from "../history/panelMessages";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass history root is missing");
const root = createRoot(element);
let timeline: HistoryTimeline | undefined;
let selectedCommit = "";
let graph: GraphViewModel | undefined;
let graphCommit: string | undefined;
let graphIdentity: { realization: string; fingerprint: string } | undefined;
let comparisonIdentities: {
  current: { realization: string; fingerprint: string };
  parent: { realization: string; fingerprint: string };
} | undefined;
let communityDetail: CommunityGraphDetail | undefined;
let communityLoading: number | null = null;
let communityError: string | undefined;
let activeCommunityRequest = "";
let activeCommunityContext: {
  requestId: string;
  commit: string;
  parent?: string | undefined;
  communityId: number;
  parentMembers: number;
  currentMembers: number;
} | undefined;
let semanticDiff: SemanticDiffReport | undefined;
let comparison: (GraphComparison & { parent: string }) | undefined;
let repositoryId = "";
let changeCounts: HistoryChangeCounts | undefined;
let revisionLoadState: "idle" | "loading" | "ready" = "idle";
let bootstrapError: string | undefined;
let loadingMore = false;
let loadMoreError: string | undefined;
let enableState: HistoryBuildState | undefined;
let timelineGeneration = 0;
const buildStates = new Map<string, HistoryBuildState>();
const operationErrors = new Map<string, HistoryOperationError>();

function postMessage(message: HistoryWebviewMessage): void {
  vscode.postMessage(message);
}

function clearRevisionPresentation(): void {
  graph = undefined;
  graphCommit = undefined;
  graphIdentity = undefined;
  comparisonIdentities = undefined;
  semanticDiff = undefined;
  comparison = undefined;
  changeCounts = undefined;
  communityDetail = undefined;
  communityLoading = null;
  communityError = undefined;
  activeCommunityRequest = "";
  activeCommunityContext = undefined;
  revisionLoadState = "idle";
}

function requestChangeCounts(commit: string): void {
  const entry = timeline?.entries.find((candidate) => candidate.commit === commit);
  if (timeline?.historyEnabled && entry?.presentationAvailable && entry.parents.length > 0) {
    postMessage({ type: "changeCounts", commit });
  }
}

function requestSelectedRevision(commit: string): void {
  const entry = timeline?.entries.find((candidate) => candidate.commit === commit);
  if (!timeline?.historyEnabled || !entry?.presentationAvailable) {
    revisionLoadState = "idle";
    return;
  }
  revisionLoadState = "loading";
  operationErrors.delete(commit);
  postMessage({ type: "loadRevision", commit });
}

function selectCommit(commit: string): void {
  if (commit === selectedCommit) return;
  selectedCommit = commit;
  clearRevisionPresentation();
  requestChangeCounts(commit);
  requestSelectedRevision(commit);
  render();
}

function acceptsCommit(commit: unknown): commit is string {
  return typeof commit === "string" && commit === selectedCommit;
}

function render(): void {
  if (bootstrapError) {
    root.render(
      <main className="history-bootstrap">
        <WorkspaceState
          kind="error"
          title="Codebase evolution is unavailable"
          description={bootstrapError}
          action={{
            label: "Retry history",
            onClick() {
              bootstrapError = undefined;
              render();
              postMessage({ type: "retryTimeline" });
            }
          }}
        />
      </main>
    );
    return;
  }
  if (!timeline) {
    root.render(
      <main className="history-bootstrap">
        <WorkspaceState
          kind="running"
          title="Loading commit history"
          description="Compass is reading reachable commits and revision graph states."
        />
      </main>
    );
    return;
  }
  root.render(
    <HistoryWorkspace
      timeline={timeline}
      selectedCommit={selectedCommit}
      revisionLoadState={revisionLoadState}
      enableState={enableState}
      loadingMore={loadingMore}
      loadMoreError={loadMoreError}
      buildStates={buildStates}
      operationError={operationErrors.get(selectedCommit)}
      onSelectCommit={selectCommit}
      graph={graph}
      graphCommit={graphCommit}
      comparison={comparison}
      communityDetail={communityDetail}
      communityLoading={communityLoading}
      communityError={communityError}
      onBackToOverview={() => {
        communityDetail = undefined;
        communityLoading = null;
        communityError = undefined;
        activeCommunityRequest = "";
        activeCommunityContext = undefined;
        render();
      }}
      onExitComparison={() => {
        postMessage({ type: "exitComparison", commit: selectedCommit });
        comparison = undefined;
        comparisonIdentities = undefined;
        semanticDiff = undefined;
        communityDetail = undefined;
        communityLoading = null;
        communityError = undefined;
        activeCommunityRequest = "";
        activeCommunityContext = undefined;
        requestChangeCounts(selectedCommit);
        render();
      }}
      semanticDiff={semanticDiff}
      changeCounts={changeCounts}
      host={{
        enableHistory() {
          enableState = { status: "requesting" };
          render();
          postMessage({ type: "enableHistory" });
        },
        loadMore() {
          if (loadingMore || !timeline?.hasMore) return;
          loadingMore = true;
          loadMoreError = undefined;
          render();
          postMessage({ type: "loadMoreTimeline" });
        },
        loadRevision(commit) {
          revisionLoadState = "loading";
          operationErrors.delete(commit);
          render();
          postMessage({ type: "loadRevision", commit });
        },
        buildRevision(commit) {
          buildStates.set(commit, { status: "requesting" });
          operationErrors.delete(commit);
          render();
          postMessage({ type: "buildRevision", commit });
        },
        compare(commit, parent) {
          operationErrors.delete(commit);
          render();
          postMessage({ type: "compare", commit, parent });
        },
        queryRevision(commit) {
          postMessage({ type: "queryRevision", commit });
        },
        loadChangeCounts(commit) {
          postMessage({ type: "changeCounts", commit });
        },
        openSource(commit, source) {
          postMessage({ type: "openSource", commit, repositoryId, source });
        },
        openCommunity(commit, communityId) {
          if (communityLoading !== null || !graphIdentity) return;
          communityLoading = communityId;
          communityError = undefined;
          activeCommunityRequest = crypto.randomUUID();
          const aggregate = comparison?.graph.nodes.find(
            (node) => node.community === communityId && node.memberCount !== undefined
          );
          if (comparison && comparisonIdentities && aggregate) {
            const hasCurrent = aggregate.change === "unchanged"
              || aggregate.evidence?.after !== undefined;
            const hasParent = aggregate.change === "unchanged"
              || aggregate.evidence?.before !== undefined;
            const currentMembers = memberCount(
              aggregate.evidence?.after,
              hasCurrent ? (aggregate.memberCount ?? 0) : 0
            );
            const parentMembers = memberCount(
              aggregate.evidence?.before,
              hasParent ? (aggregate.memberCount ?? 0) : 0
            );
            activeCommunityContext = {
              requestId: activeCommunityRequest,
              commit,
              parent: comparison.parent,
              communityId,
              parentMembers,
              currentMembers
            };
            postMessage({
              type: "compareCommunity",
              requestId: activeCommunityRequest,
              commit,
              parent: comparison.parent,
              currentIdentity: comparisonIdentities.current,
              parentIdentity: comparisonIdentities.parent,
              communityId,
              hasCurrent,
              hasParent
            });
            render();
            return;
          }
          activeCommunityContext = {
            requestId: activeCommunityRequest,
            commit,
            communityId,
            parentMembers: 0,
            currentMembers: aggregate?.memberCount ?? 0
          };
          postMessage({
            type: "openCommunity",
            requestId: activeCommunityRequest,
            commit,
            realization: graphIdentity.realization,
            fingerprint: graphIdentity.fingerprint,
            communityId
          });
          render();
        }
      }}
    />
  );
}

window.addEventListener("message", (event: MessageEvent<HistoryHostMessage>) => {
  const message = event.data;
  if (message?.type === "timeline") {
    if (message.generation < timelineGeneration) return;
    const parsed = HistoryTimelineSchema.safeParse(message.timeline);
    if (parsed.success) {
      timelineGeneration = message.generation;
      timeline = parsed.data;
      bootstrapError = undefined;
      loadingMore = false;
      loadMoreError = undefined;
      enableState = undefined;
      repositoryId = message.repositoryId;
      const retainedCommit = timeline.entries.some((entry) => entry.commit === selectedCommit)
        ? selectedCommit
        : "";
      const nextCommit = retainedCommit
        || (timeline.entries.some((entry) => entry.commit === timeline?.selectedHead)
          ? timeline.selectedHead
          : timeline.entries[0]?.commit)
        || "";
      if (nextCommit !== selectedCommit) {
        selectedCommit = nextCommit;
        clearRevisionPresentation();
      }
      requestChangeCounts(nextCommit);
      if (nextCommit && graphCommit !== nextCommit) requestSelectedRevision(nextCommit);
    }
  } else if (message?.type === "timelinePage") {
    const parsed = HistoryTimelineSchema.safeParse(message.timeline);
    if (parsed.success
      && timeline
      && message.repositoryId === repositoryId
      && message.generation === timelineGeneration) {
      const loaded = new Set(timeline.entries.map((entry) => entry.commit));
      timeline = {
        ...parsed.data,
        entries: [
          ...timeline.entries,
          ...parsed.data.entries.filter((entry) => !loaded.has(entry.commit))
        ]
      };
      loadingMore = false;
      loadMoreError = undefined;
    }
  } else if (message?.type === "timelinePageError") {
    if (message.generation === timelineGeneration) {
      loadingMore = false;
      loadMoreError = message.message;
    }
  } else if (message?.type === "bootstrapError") {
    bootstrapError = message.message;
    timeline = undefined;
    clearRevisionPresentation();
  } else if (message?.type === "enableRunning") {
    enableState = { status: "running" };
  } else if (message?.type === "enableSucceeded" || message?.type === "enableCancelled") {
    enableState = undefined;
  } else if (message?.type === "enableFailed") {
    enableState = { status: "failed", message: message.message };
  } else if (message?.type === "graph") {
    if (!acceptsCommit(message.commit)) return;
    const parsed = GraphViewModelSchema.safeParse(message.graph);
    if (parsed.success) {
      graph = parsed.data;
      graphCommit = message.commit;
      graphIdentity = {
        realization: message.realization,
        fingerprint: message.fingerprint
      };
      comparisonIdentities = undefined;
      communityDetail = undefined;
      communityLoading = null;
      communityError = undefined;
      activeCommunityRequest = "";
      activeCommunityContext = undefined;
      operationErrors.delete(message.commit);
      revisionLoadState = "ready";
    }
  } else if (message?.type === "communityGraph") {
    if (!acceptsCommit(message.commit)) return;
    const parsed = GraphViewModelSchema.safeParse(message.graph);
    if (parsed.success
      && message.requestId === activeCommunityRequest
      && message.commit === graphCommit) {
      communityDetail = {
        communityId: message.communityId,
        model: parsed.data
      };
      communityLoading = null;
      communityError = undefined;
    }
  } else if (message?.type === "communityError") {
    if (!acceptsCommit(message.commit)) return;
    if (message.requestId === activeCommunityRequest) {
      communityLoading = null;
      communityError = message.message;
    }
  } else if (message?.type === "comparison") {
    if (!acceptsCommit(message.commit)) return;
    const current = GraphViewModelSchema.safeParse(message.currentGraph);
    const parent = GraphViewModelSchema.safeParse(message.parentGraph);
    const report = SemanticDiffReportSchema.safeParse(message.semanticDiff);
    if (current.success && parent.success && report.success) {
      const parsedCounts = message.counts
        ? HistoryChangeCountsSchema.safeParse(message.counts)
        : undefined;
      const structuralCounts = parsedCounts?.success ? parsedCounts.data : undefined;
      graph = current.data;
      graphCommit = message.commit;
      graphIdentity = {
        realization: message.realization,
        fingerprint: message.fingerprint
      };
      comparisonIdentities = {
        current: {
          realization: message.realization,
          fingerprint: message.fingerprint
        },
        parent: {
          realization: message.parentRealization,
          fingerprint: message.parentFingerprint
        }
      };
      communityDetail = undefined;
      communityLoading = null;
      communityError = undefined;
      activeCommunityRequest = "";
      activeCommunityContext = undefined;
      const visibleComparison = compareGraphs(parent.data, current.data);
      const structuralComparison = comparisonFromSemanticDiff(
        report.data,
        current.data,
        parent.data
      ) ?? visibleComparison;
      comparison = {
        ...structuralComparison,
        parent: message.parent
      };
      semanticDiff = report.data;
      if (structuralCounts) changeCounts = structuralCounts;
      operationErrors.delete(message.commit);
      revisionLoadState = "ready";
    } else {
      comparison = undefined;
      comparisonIdentities = undefined;
      semanticDiff = undefined;
      changeCounts = undefined;
      communityDetail = undefined;
      communityLoading = null;
      communityError = undefined;
      activeCommunityRequest = "";
      activeCommunityContext = undefined;
      if (current.success) {
        graph = current.data;
        graphCommit = message.commit;
        graphIdentity = {
          realization: message.realization,
          fingerprint: message.fingerprint
        };
        revisionLoadState = "ready";
      }
      operationErrors.set(message.commit, {
        operation: "Compare revisions",
        message: "Compass returned an unsupported or malformed versioned graph comparison."
      });
    }
  } else if (message?.type === "communityComparison") {
    const context = activeCommunityContext;
    if (!context
      || message.requestId !== activeCommunityRequest
      || message.requestId !== context.requestId
      || message.commit !== selectedCommit
      || message.commit !== context.commit
      || message.parent !== comparison?.parent
      || message.parent !== context.parent
      || message.communityId !== context.communityId) {
      return;
    }
    const current = message.currentGraph
      ? GraphViewModelSchema.safeParse(message.currentGraph)
      : undefined;
    const parent = message.parentGraph
      ? GraphViewModelSchema.safeParse(message.parentGraph)
      : undefined;
    if ((current && !current.success) || (parent && !parent.success) || (!current && !parent)) {
      communityLoading = null;
      communityError = "Compass returned an invalid community comparison.";
    } else {
      const reference = current?.success
        ? current.data
        : parent?.success
          ? parent.data
          : comparison.graph;
      const currentGraph = current?.success
        ? current.data
        : emptyGraph(reference, "Current community");
      const parentGraph = parent?.success
        ? parent.data
        : emptyGraph(reference, "Parent community");
      const detail = compareGraphs(parentGraph, currentGraph);
      const bounded = currentGraph.nodes.length < context.currentMembers
        || parentGraph.nodes.length < context.parentMembers;
      communityDetail = {
        communityId: message.communityId,
        model: detail.graph,
        ...(bounded
          ? {
              bounded: {
                limit: message.nodeLimit,
                parentMembers: context.parentMembers,
                currentMembers: context.currentMembers
              }
            }
          : {})
      };
      communityLoading = null;
      communityError = undefined;
    }
  } else if (message?.type === "communityComparisonError") {
    const context = activeCommunityContext;
    if (!context
      || message.requestId !== activeCommunityRequest
      || message.requestId !== context.requestId
      || message.commit !== selectedCommit
      || message.parent !== comparison?.parent
      || message.communityId !== context.communityId) {
      return;
    }
    communityLoading = null;
    communityError = message.message;
  } else if (message?.type === "changeCounts") {
    if (!acceptsCommit(message.commit)) return;
    const parsed = HistoryChangeCountsSchema.safeParse(message.counts);
    if (parsed.success) changeCounts = parsed.data;
  } else if (message?.type === "buildRunning") {
    buildStates.set(message.commit, { status: "running" });
  } else if (message?.type === "buildSucceeded") {
    buildStates.delete(message.commit);
    operationErrors.delete(message.commit);
  } else if (message?.type === "buildFailed") {
    buildStates.set(message.commit, { status: "failed", message: message.message });
  } else if (message?.type === "buildCancelled") {
    buildStates.delete(message.commit);
  } else if (message?.type === "error") {
    if (message.commit !== undefined && message.commit !== selectedCommit) return;
    const commit = message.commit ?? selectedCommit;
    if (commit) {
      operationErrors.set(commit, {
        operation: message.operation,
        message: message.message
      });
      if (message.operation === "Load graph") revisionLoadState = "idle";
    }
  } else {
    return;
  }
  render();
});

render();
postMessage({ type: "ready" });

function memberCount(
  snapshot: Record<string, unknown> | undefined,
  fallback: number
): number {
  return typeof snapshot?.memberCount === "number" ? snapshot.memberCount : fallback;
}

function emptyGraph(reference: GraphViewModel, title: string): GraphViewModel {
  return {
    ...reference,
    title,
    stats: {
      ...reference.stats,
      nodes: 0,
      edges: 0,
      communities: 0,
      aggregated: false
    },
    nodes: [],
    edges: [],
    communities: [],
    hyperedges: []
  };
}
