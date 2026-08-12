import { createRoot, type Root } from "react-dom/client";
import {
  VisualizationWorkbench,
  WORKBENCH_SCHEMA,
  codeQueryGraphViewModel,
  type CodeQueryResponse,
  type GraphViewModel,
  type InspectorLayout,
  type WorkbenchModel,
  type WorkbenchView
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
let queryResult: CodeQueryResponse | undefined;
let communityDetail: { communityId: number; model: GraphViewModel } | undefined;
let communityLoading: number | null = null;
let communityError: string | undefined;
let activeCommunityRequest = "";
let loadingCopy: GraphLoadingCopy | undefined;

function resetGraphState(): void {
  overview = undefined;
  queryResult = undefined;
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
  const workbench = workbenchModel(overview, queryResult, repositoryId);
  root.render(
    <VisualizationWorkbench
      workbench={workbench}
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
        queryNode(operation, symbol) {
          vscode.postMessage({
            type: "runCodeQuery",
            repositoryId,
            operation,
            symbol
          });
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

function workbenchModel(
  code: GraphViewModel,
  query: CodeQueryResponse | undefined,
  identity: string
): WorkbenchModel {
  const views: WorkbenchView[] = [{
    id: "code",
    title: "Code graph",
    description: "Repository structure, ownership, and relationships",
    coverage: graphCoverage(code),
    kind: "code",
    model: code,
    communityDetails: {}
  }];
  if (query) {
    const root = query.nodes[0]?.name ?? query.nodes[0]?.id ?? "query";
    const title = queryTitle(query, root);
    if (query.operation === "impact") {
      views.push({
        id: "impact",
        title,
        description: "Inbound code paths that can be affected by a change",
        coverage: queryCoverage(query),
        kind: "impact",
        root,
        result: query
      });
    } else {
      views.push({
        id: `query-${query.operation}`,
        title,
        description: "A focused projection returned by the typed code-query engine",
        coverage: queryCoverage(query),
        kind: "affected",
        root,
        relations: [...new Set(query.edges.map((edge) => edge.kind))].sort(),
        depth: query.limits.maxDepth,
        model: codeQueryGraphViewModel(query, title)
      });
    }
  }
  return {
    schema: WORKBENCH_SCHEMA,
    title: code.title,
    graphIdentity: `repository:${identity}`,
    defaultView: query ? views[1]!.id : "code",
    views
  };
}

function graphCoverage(model: GraphViewModel) {
  return {
    status: model.stats.aggregated ? "summary" as const : "complete" as const,
    truncated: false,
    nodes: model.stats.nodes,
    edges: model.stats.edges,
    limitations: model.stats.aggregated
      ? ["The repository overview is aggregated by community."]
      : []
  };
}

function queryCoverage(query: CodeQueryResponse) {
  return {
    status: query.truncated ? "partial" as const : "complete" as const,
    truncated: query.truncated,
    nodes: query.nodes.length,
    edges: query.edges.length,
    limitations: query.truncated
      ? ["The query reached its configured node or edge bound."]
      : []
  };
}

function queryTitle(query: CodeQueryResponse, root: string): string {
  const label = query.operation === "impact"
    ? "Impact"
    : query.operation === "callers"
      ? "Callers"
      : query.operation === "callees" ? "Callees" : humanize(query.operation);
  return `${label} · ${root}`;
}

function humanize(value: string): string {
  return value.replaceAll("_", " ").replace(/^./, (character) => character.toUpperCase());
}

window.addEventListener("message", (event) => {
  const parsed = HostToGraphMessageSchema.safeParse(event.data);
  if (!parsed.success) return;
  if (parsed.data.type === "error") {
    renderError(parsed.data.message);
    return;
  }
  if (parsed.data.type === "graphLoadStatus") {
    loadingCopy = parsed.data.phase === "snapshotting"
      ? {
          eyebrow: `Compass code graph · ${formatBytes(parsed.data.graphBytes)}`,
          title: "Preparing a large code graph",
          steps: ["Securing snapshot", "Building overview", "Opening explorer"],
          activeStep: 0
        }
      : {
          eyebrow: `Compass code graph · ${formatBytes(parsed.data.graphBytes)}`,
          title: "Preparing a large code graph",
          steps: ["Snapshot ready", "Building overview", "Opening explorer"],
          activeStep: 1
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
  } else if (parsed.data.type === "codeQueryResult") {
    if (parsed.data.repositoryId !== repositoryId) return;
    queryResult = parsed.data.result;
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
