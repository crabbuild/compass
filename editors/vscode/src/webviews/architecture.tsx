import { createRoot } from "react-dom/client";
import { ArchitectureFlow, CallflowViewModelSchema } from "@compass/viewer";
import { GraphLoadingState, type GraphLoadingCopy } from "./GraphLoadingState";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass architecture root is missing");
const root = createRoot(element);

const ARCHITECTURE_LOADING_COPY: GraphLoadingCopy = {
  eyebrow: "Compass architecture",
  title: "Deriving architecture flow",
  steps: ["Reading graph", "Deriving subsystem flows", "Preparing symbol index"]
};

function renderLoading(): void {
  root.render(
    <GraphLoadingState
      state={{ kind: "loading" }}
      variant="architecture"
      loadingCopy={ARCHITECTURE_LOADING_COPY}
      onRetry={() => vscode.postMessage({ type: "retry" })}
      onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
    />
  );
}

function renderError(message: string): void {
  root.render(
    <GraphLoadingState
      state={{ kind: "error", message }}
      variant="architecture"
      loadingCopy={ARCHITECTURE_LOADING_COPY}
      onRetry={() => {
        renderLoading();
        vscode.postMessage({ type: "retry" });
      }}
      onShowOutput={() => vscode.postMessage({ type: "showOutput" })}
    />
  );
}

window.addEventListener("message", (event) => {
  if (event.data?.type === "error") {
    renderError(
      typeof event.data.message === "string"
        ? event.data.message
        : "Compass could not derive the architecture flow."
    );
    return;
  }
  if (event.data?.type !== "hydrate") return;
  const parsed = CallflowViewModelSchema.safeParse(event.data.model);
  if (!parsed.success || typeof event.data.repositoryId !== "string") return;
  const repositoryId = event.data.repositoryId;
  root.render(
    <ArchitectureFlow
      model={parsed.data}
      host={{
        openSource(file) {
          vscode.postMessage({ type: "openSource", repositoryId, file });
        }
      }}
    />
  );
});
renderLoading();
vscode.postMessage({ type: "ready" });
