import { createRoot } from "react-dom/client";
import {
  QueryWorkspace,
  type QueryOutput,
  type QueryRun
} from "@compass/viewer";

declare function acquireVsCodeApi(): { postMessage(message: unknown): void };
const vscode = acquireVsCodeApi();
const element = document.getElementById("root");
if (!element) throw new Error("Compass query root is missing");
const root = createRoot(element);
let runs: QueryRun[] = [];
let activeRunId: string | undefined;
let revision: string | undefined;
let sequence = 0;

function render(): void {
  root.render(
    <QueryWorkspace
      runs={runs}
      activeRunId={activeRunId}
      revision={revision}
      host={{
        execute(request) {
          const id = `${request.command}-${++sequence}`;
          const run: QueryRun = { id, request, status: "running" };
          runs = [...runs, run].slice(-8);
          activeRunId = id;
          render();
          vscode.postMessage({ type: "execute", request: { ...request, id } });
        },
        cancel(runId) {
          updateRun(runId, { status: "cancelled" });
          render();
          vscode.postMessage({ type: "cancel", runId });
        },
        selectRun(runId) {
          activeRunId = runId;
          render();
        },
        closeRun(runId) {
          const index = runs.findIndex((run) => run.id === runId);
          const closing = runs[index];
          if (closing?.status === "running") {
            vscode.postMessage({ type: "cancel", runId });
          }
          runs = runs.filter((run) => run.id !== runId);
          if (activeRunId === runId) {
            activeRunId = runs[Math.min(index, runs.length - 1)]?.id;
          }
          render();
        },
        openSource(source) {
          vscode.postMessage({ type: "openSource", source });
        },
        openGraph() {
          vscode.postMessage({ type: "openGraph" });
        }
      }}
    />
  );
}

function updateRun(runId: string, update: Partial<QueryRun>): void {
  runs = runs.map((run) => run.id === runId ? { ...run, ...update } : run);
}

window.addEventListener("message", (event) => {
  const runId = typeof event.data?.runId === "string" ? event.data.runId : undefined;
  if (event.data?.type === "state") {
    revision = event.data.revision;
  } else if (event.data?.type === "result" && runId) {
    updateRun(runId, {
      status: "success",
      output: event.data.output as QueryOutput,
      durationMs: Number(event.data.durationMs)
    });
    activeRunId = runId;
    revision = event.data.revision;
  } else if (event.data?.type === "error" && runId) {
    updateRun(runId, { status: "error", error: String(event.data.message) });
    activeRunId = runId;
    revision = event.data.revision;
  } else if (event.data?.type === "cancelled" && runId) {
    updateRun(runId, { status: "cancelled" });
  } else {
    return;
  }
  render();
});
render();
vscode.postMessage({ type: "ready" });
