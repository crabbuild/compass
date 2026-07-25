import { createRoot } from "react-dom/client";
import {
  HistoryTimelineSchema,
  HistoryChangeCountsSchema,
  HistoryWorkspace,
  GraphViewModelSchema,
  compareGraphs,
  type GraphViewModel,
  type HistoryChangeCounts,
  type HistoryTimeline
} from "@compass/viewer";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass history root is missing");
const root = createRoot(element);
let timeline: HistoryTimeline | undefined;
let graph: GraphViewModel | undefined;
let graphCommit: string | undefined;
let communityDetail: { communityId: number; model: GraphViewModel } | undefined;
let communityLoading: number | null = null;
let communityError: string | undefined;
let activeCommunityRequest = "";
let semanticDiff: unknown;
let repositoryId = "";
let changeCounts: HistoryChangeCounts | undefined;

function render(): void {
  if (!timeline) return;
  root.render(
    <HistoryWorkspace
      timeline={timeline}
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
          vscode.postMessage({ type: "loadRevision", commit });
        },
        buildRevision(commit) {
          vscode.postMessage({ type: "buildRevision", commit });
        },
        compare(commit, parent) {
          vscode.postMessage({ type: "compare", commit, parent });
        },
        queryRevision(commit) {
          vscode.postMessage({ type: "queryRevision", commit });
        },
        loadChangeCounts(commit) {
          vscode.postMessage({ type: "changeCounts", commit });
        },
        openSource(source) {
          vscode.postMessage({ type: "openSource", repositoryId, source });
        },
        openCommunity(commit, communityId) {
          if (communityLoading !== null) return;
          communityLoading = communityId;
          communityError = undefined;
          activeCommunityRequest = crypto.randomUUID();
          vscode.postMessage({
            type: "openCommunity",
            requestId: activeCommunityRequest,
            commit,
            communityId
          });
          render();
        }
      }}
    />
  );
}
window.addEventListener("message", (event) => {
  if (event.data?.type === "timeline") {
    const parsed = HistoryTimelineSchema.safeParse(event.data.timeline);
    if (parsed.success) {
      timeline = parsed.data;
      if (typeof event.data.repositoryId === "string") {
        repositoryId = event.data.repositoryId;
      }
    }
  } else if (event.data?.type === "graph") {
    const parsed = GraphViewModelSchema.safeParse(event.data.graph);
    if (parsed.success) {
      graph = parsed.data;
      graphCommit = typeof event.data.commit === "string" ? event.data.commit : undefined;
      communityDetail = undefined;
      communityLoading = null;
      communityError = undefined;
      activeCommunityRequest = "";
    }
  } else if (event.data?.type === "communityGraph") {
    const parsed = GraphViewModelSchema.safeParse(event.data.graph);
    if (parsed.success
      && event.data.requestId === activeCommunityRequest
      && event.data.commit === graphCommit) {
      communityDetail = {
        communityId: event.data.communityId,
        model: parsed.data
      };
      communityLoading = null;
      communityError = undefined;
    }
  } else if (event.data?.type === "communityError") {
    if (event.data.requestId === activeCommunityRequest) {
      communityLoading = null;
      communityError = String(event.data.message);
    }
  } else if (event.data?.type === "comparison") {
    const current = GraphViewModelSchema.safeParse(event.data.currentGraph);
    const parent = GraphViewModelSchema.safeParse(event.data.parentGraph);
    if (current.success && parent.success) {
      graph = current.data;
      graphCommit = typeof event.data.commit === "string" ? event.data.commit : undefined;
      communityDetail = undefined;
      communityLoading = null;
      communityError = undefined;
      activeCommunityRequest = "";
      semanticDiff = {
        structural: compareGraphs(parent.data, current.data),
        semantic: event.data.semanticDiff
      };
    }
  } else if (event.data?.type === "changeCounts") {
    const parsed = HistoryChangeCountsSchema.safeParse(event.data.counts);
    if (parsed.success) changeCounts = parsed.data;
  } else if (event.data?.type === "error") {
    semanticDiff = { error: String(event.data.message) };
  } else {
    return;
  }
  render();
});
vscode.postMessage({ type: "ready" });
