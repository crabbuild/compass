import { createRoot } from "react-dom/client";
import { CallGraph, CallGraphResponseSchema, mergeExpansion, type CallGraphResponse } from "@compass/viewer";
import { GraphLoadingState, type GraphLoadingCopy } from "./GraphLoadingState";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass call graph root is missing");
const root = createRoot(element);
let graph: CallGraphResponse | undefined;
let repositoryId = "";

const CALL_GRAPH_LOADING_COPY: GraphLoadingCopy = {
  eyebrow: "Compass call graph",
  title: "Resolving the function under your cursor",
  steps: ["Locating symbol", "Tracing callers", "Tracing callees"]
};

function renderLoading(): void {
  root.render(
    <GraphLoadingState
      state={{ kind: "loading" }}
      loadingCopy={CALL_GRAPH_LOADING_COPY}
      onRetry={() => vscode.postMessage({ type: "retry" })}
      onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
    />
  );
}

function renderError(message: string): void {
  root.render(
    <GraphLoadingState
      state={{ kind: "error", message }}
      loadingCopy={CALL_GRAPH_LOADING_COPY}
      onRetry={() => {
        renderLoading();
        vscode.postMessage({ type: "retry" });
      }}
      onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
    />
  );
}

function render(): void {
  if (!graph) return;
  root.render(
    <CallGraph
      graph={graph}
      host={{
        openSource(source) {
          vscode.postMessage({ type: "openSource", repositoryId, source });
        },
        expand(symbol, direction, depth) {
          vscode.postMessage({ type: "expand", symbol, direction, depth });
        },
        changeDirection(direction) {
          renderLoading();
          vscode.postMessage({ type: "changeDirection", direction });
        }
      }}
    />
  );
}

window.addEventListener("message", (event) => {
  if (event.data?.type === "error") {
    renderError(
      typeof event.data.message === "string"
        ? event.data.message
        : "Compass could not resolve this function."
    );
    return;
  }
  if (!["hydrateCallGraph", "mergeCallGraph"].includes(event.data?.type)) return;
  const parsed = CallGraphResponseSchema.safeParse(event.data.graph);
  if (!parsed.success || typeof event.data.repositoryId !== "string") return;
  repositoryId = event.data.repositoryId;
  graph = graph && event.data.type === "mergeCallGraph"
    ? mergeExpansion(graph, parsed.data)
    : parsed.data;
  render();
});
renderLoading();
vscode.postMessage({ type: "ready" });
