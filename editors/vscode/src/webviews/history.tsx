import { createRoot } from "react-dom/client";
import {
  HistoryTimelineSchema,
  HistoryChangeCountsSchema,
  HistoryWorkspace,
  GraphViewModelSchema,
  compareGraphs,
  type GraphViewModel,
  type HistoryBuildState,
  type HistoryChangeCounts,
  type HistoryOperationError,
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
let communityDetail: { communityId: number; model: GraphViewModel } | undefined;
let communityLoading: number | null = null;
let communityError: string | undefined;
let activeCommunityRequest = "";
let semanticDiff: unknown;
let repositoryId = "";
let changeCounts: HistoryChangeCounts | undefined;
const buildStates = new Map<string, HistoryBuildState>();
const operationErrors = new Map<string, HistoryOperationError>();

function postMessage(message: HistoryWebviewMessage): void {
  vscode.postMessage(message);
}

function clearRevisionPresentation(): void {
  graph = undefined;
  graphCommit = undefined;
  graphIdentity = undefined;
  semanticDiff = undefined;
  changeCounts = undefined;
  communityDetail = undefined;
  communityLoading = null;
  communityError = undefined;
  activeCommunityRequest = "";
}

function requestChangeCounts(commit: string): void {
  const entry = timeline?.entries.find((candidate) => candidate.commit === commit);
  if (entry?.presentationAvailable && entry.parents.length > 0) {
    postMessage({ type: "changeCounts", commit });
  }
}

function selectCommit(commit: string): void {
  if (commit === selectedCommit) return;
  selectedCommit = commit;
  clearRevisionPresentation();
  requestChangeCounts(commit);
  render();
}

function acceptsCommit(commit: unknown): commit is string {
  return typeof commit === "string" && commit === selectedCommit;
}

function render(): void {
  if (!timeline) return;
  root.render(
    <HistoryWorkspace
      timeline={timeline}
      selectedCommit={selectedCommit}
      buildState={buildStates.get(selectedCommit)}
      operationError={operationErrors.get(selectedCommit)}
      onSelectCommit={selectCommit}
      graph={graph}
      graphCommit={graphCommit}
      communityDetail={communityDetail}
      communityLoading={communityLoading}
      communityError={communityError}
      onBackToOverview={() => {
        communityDetail = undefined;
        communityLoading = null;
        communityError = undefined;
        activeCommunityRequest = "";
        render();
      }}
      semanticDiff={semanticDiff}
      changeCounts={changeCounts}
      host={{
        loadRevision(commit) {
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
    const parsed = HistoryTimelineSchema.safeParse(message.timeline);
    if (parsed.success) {
      timeline = parsed.data;
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
    }
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
      communityDetail = undefined;
      communityLoading = null;
      communityError = undefined;
      activeCommunityRequest = "";
      operationErrors.delete(message.commit);
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
    if (current.success && parent.success) {
      graph = current.data;
      graphCommit = message.commit;
      graphIdentity = {
        realization: message.realization,
        fingerprint: message.fingerprint
      };
      communityDetail = undefined;
      communityLoading = null;
      communityError = undefined;
      activeCommunityRequest = "";
      semanticDiff = {
        structural: compareGraphs(parent.data, current.data),
        semantic: message.semanticDiff
      };
      operationErrors.delete(message.commit);
    }
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
    }
  } else {
    return;
  }
  render();
});

postMessage({ type: "ready" });
