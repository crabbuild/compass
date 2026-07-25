import { createRoot } from "react-dom/client";
import { CallGraph, CallGraphResponseSchema, mergeExpansion, type CallGraphResponse } from "@compass/viewer";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass call graph root is missing");
const root = createRoot(element);
let graph: CallGraphResponse | undefined;
let repositoryId = "";

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
        }
      }}
    />
  );
}

window.addEventListener("message", (event) => {
  if (event.data?.type === "error") {
    root.render(<main className="grid min-h-screen place-items-center p-8">{event.data.message}</main>);
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
vscode.postMessage({ type: "ready" });
