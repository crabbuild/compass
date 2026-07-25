import { createRoot } from "react-dom/client";
import { QueryWorkspace, type QueryResult } from "@compass/viewer";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass query root is missing");
const root = createRoot(element);
let running = false;
let result: QueryResult | undefined;
let error: string | undefined;
let revision: string | undefined;

function render(): void {
  root.render(
    <QueryWorkspace
      running={running}
      result={result}
      error={error}
      revision={revision}
      host={{
        execute(request) {
          vscode.postMessage({ type: "execute", request });
        },
        cancel() {
          vscode.postMessage({ type: "cancel" });
        }
      }}
    />
  );
}
window.addEventListener("message", (event) => {
  if (event.data?.type === "state") {
    running = Boolean(event.data.running);
    revision = event.data.revision;
  } else if (event.data?.type === "result") {
    running = false;
    error = undefined;
    result = event.data.result;
    revision = event.data.revision;
  } else if (event.data?.type === "error") {
    running = false;
    result = undefined;
    error = String(event.data.message);
    revision = event.data.revision;
  } else {
    return;
  }
  render();
});
render();
vscode.postMessage({ type: "ready" });
