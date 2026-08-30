import { createRoot } from "react-dom/client";
import {
  QueryWorkspace,
  type QueryCompletion,
  type QueryCompletionRequest,
  type QueryHost,
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
let completionSequence = 0;
const pendingCompletions = new Map<string, {
  resolve(items: QueryCompletion[]): void;
  reject(error: Error): void;
  timeout: number;
  signal?: AbortSignal | undefined;
  abortListener?: (() => void) | undefined;
}>();

const host: QueryHost = {
  complete(request: QueryCompletionRequest, signal?: AbortSignal) {
    const id = `completion-${++completionSequence}`;
    return new Promise<QueryCompletion[]>((resolve, reject) => {
      const timeout = window.setTimeout(() => {
        const pending = takePendingCompletion(id);
        if (!pending) return;
        vscode.postMessage({ type: "cancelCompletion", requestId: id });
        pending.reject(new Error("Code graph completion timed out"));
      }, 5000);
      const abortListener = () => {
        const pending = takePendingCompletion(id);
        if (!pending) return;
        vscode.postMessage({ type: "cancelCompletion", requestId: id });
        pending.reject(new Error("Code graph completion cancelled"));
      };
      pendingCompletions.set(id, {
        resolve,
        reject,
        timeout,
        signal,
        abortListener
      });
      signal?.addEventListener("abort", abortListener, { once: true });
      if (signal?.aborted) {
        abortListener();
        return;
      }
      vscode.postMessage({ type: "complete", request: { ...request, id } });
    });
  },
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
};

function render(): void {
  root.render(
    <QueryWorkspace
      runs={runs}
      activeRunId={activeRunId}
      revision={revision}
      host={host}
    />
  );
}

function updateRun(runId: string, update: Partial<QueryRun>): void {
  runs = runs.map((run) => run.id === runId ? { ...run, ...update } : run);
}

window.addEventListener("message", (event) => {
  const requestId = typeof event.data?.requestId === "string"
    ? event.data.requestId
    : undefined;
  if (requestId && ["completions", "completionError", "completionCancelled"]
    .includes(event.data?.type)) {
    const pending = pendingCompletions.get(requestId);
    if (!pending) return;
    takePendingCompletion(requestId);
    if (event.data.type === "completions") {
      const items = parseCompletionItems(event.data.items);
      if (items) pending.resolve(items);
      else pending.reject(new Error("Invalid code graph completion response"));
    } else if (event.data.type === "completionCancelled") {
      pending.resolve([]);
    } else {
      pending.reject(new Error(typeof event.data.message === "string"
        ? event.data.message
        : "Code graph completion failed"));
    }
    return;
  }
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

function takePendingCompletion(requestId: string) {
  const pending = pendingCompletions.get(requestId);
  if (!pending) return undefined;
  pendingCompletions.delete(requestId);
  window.clearTimeout(pending.timeout);
  if (pending.signal && pending.abortListener) {
    pending.signal.removeEventListener("abort", pending.abortListener);
  }
  return pending;
}

function parseCompletionItems(value: unknown): QueryCompletion[] | undefined {
  if (!Array.isArray(value) || value.length > 8) return undefined;
  const items: QueryCompletion[] = [];
  for (const candidate of value) {
    if (typeof candidate !== "object" || candidate === null) return undefined;
    const item = candidate as Record<string, unknown>;
    if (!boundedString(item.nodeId, 512)
      || !boundedString(item.label, 512)
      || !boundedString(item.insertText, 512)
      || !boundedString(item.detail, 240)) return undefined;
    items.push({
      nodeId: item.nodeId,
      label: item.label,
      insertText: item.insertText,
      detail: item.detail
    });
  }
  return items;
}

function boundedString(value: unknown, limit: number): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= limit;
}

render();
vscode.postMessage({ type: "ready" });
