import { createRoot, type Root } from "react-dom/client";
import {
  CompassGraph,
  type GraphViewModel,
  type InspectorLayout
} from "@compass/viewer";
import { HostToGraphMessageSchema } from "../transport/messages";
import {
  GraphLoadingState,
  type GraphLoadingCopy
} from "./GraphLoadingState";

type WebviewState = {
  inspector?: InspectorLayout;
};
declare function acquireVsCodeApi(): {
  postMessage(message: unknown): void;
  getState(): WebviewState | undefined;
  setState(state: WebviewState): void;
};

const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass graph root is missing");
const root: Root = createRoot(element);
let repositoryId = "";
let overview: GraphViewModel | undefined;
let communityDetail: { communityId: number; model: GraphViewModel } | undefined;
let communityLoading: number | null = null;
let communityError: string | undefined;
let activeCommunityRequest = "";
let loadingCopy: GraphLoadingCopy | undefined;

function resetGraphState(): void {
  overview = undefined;
  communityDetail = undefined;
  communityLoading = null;
  communityError = undefined;
  activeCommunityRequest = "";
}

function retryHydration(): void {
  resetGraphState();
  renderLoading();
  vscode.postMessage({ type: "retry" });
}

function renderLoading(): void {
  root.render(
    <GraphLoadingState
      state={{ kind: "loading" }}
      {...(loadingCopy ? { loadingCopy } : {})}
      onRetry={retryHydration}
      onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
    />
  );
}

function renderError(message: string): void {
  root.render(
    <GraphLoadingState
      state={{ kind: "error", message }}
      onRetry={retryHydration}
      onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
    />
  );
}

function renderGraph(): void {
  if (!overview) return;
  root.render(
    <CompassGraph
      model={overview}
      communityDetail={communityDetail}
      communityLoading={communityLoading}
      communityError={communityError}
      onBackToOverview={() => {
        communityDetail = undefined;
        communityLoading = null;
        communityError = undefined;
        activeCommunityRequest = "";
        renderGraph();
      }}
      initialInspectorLayout={vscode.getState()?.inspector}
      onInspectorLayoutChange={(inspector) => vscode.setState({
        ...vscode.getState(),
        inspector
      })}
      host={{
        openSource(source) {
          vscode.postMessage({ type: "openSource", repositoryId, source });
        },
        openCommunity(communityId) {
          if (communityLoading !== null) return;
          communityLoading = communityId;
          communityError = undefined;
          activeCommunityRequest = crypto.randomUUID();
          vscode.postMessage({
            type: "openCommunity",
            requestId: activeCommunityRequest,
            repositoryId,
            communityId
          });
          renderGraph();
        }
      }}
    />
  );
}

window.addEventListener("message", (event) => {
  const parsed = HostToGraphMessageSchema.safeParse(event.data);
  if (!parsed.success) return;
  if (parsed.data.type === "error") {
    renderError(parsed.data.message);
    return;
  }
  if (parsed.data.type === "graphLoadStatus") {
    loadingCopy = {
      eyebrow: `Compass code graph · ${formatBytes(parsed.data.graphBytes)}`,
      title: "Preparing a large code graph",
      steps: parsed.data.phase === "snapshotting"
        ? ["Securing snapshot", "Building overview", "Opening explorer"]
        : ["Snapshot ready", "Building overview", "Opening explorer"]
    };
    renderLoading();
    return;
  }
  if (parsed.data.type === "hydrateGraph") {
    repositoryId = parsed.data.repositoryId;
    overview = parsed.data.model;
    communityDetail = undefined;
    communityLoading = null;
    communityError = undefined;
    activeCommunityRequest = "";
  } else if (parsed.data.requestId === activeCommunityRequest) {
    communityLoading = null;
    if (parsed.data.type === "communityGraph") {
      communityDetail = {
        communityId: parsed.data.communityId,
        model: parsed.data.model
      };
      communityError = undefined;
    } else {
      communityError = parsed.data.message;
    }
  }
  renderGraph();
});

renderLoading();
vscode.postMessage({ type: "ready" });

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
