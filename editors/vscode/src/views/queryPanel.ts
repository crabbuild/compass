import { randomUUID } from "node:crypto";
import * as vscode from "vscode";
import {
  CodeQueryResponseSchema,
  SourceLocationSchema,
  type QuerySubmission
} from "@compass/viewer";
import {
  buildAskArgs,
  buildCompletionArgs,
  buildCqlArgs,
  buildExplainArgs
} from "../commands/queryArguments";
import type { RepositorySession } from "../workspace/repositorySession";
import { queryFailureMessage } from "./queryExecution";
import {
  graphCompletionItems,
  validGraphCompletionNodeId,
  validGraphCompletionTerm
} from "./queryCompletions";
import { openGraphSource } from "./sourceNavigation";

export async function openQueryPanel(
  context: vscode.ExtensionContext,
  session: RepositorySession,
  revision?: string
): Promise<void> {
  const panel = vscode.window.createWebviewPanel(
    "compass.query",
    "Compass Query",
    vscode.ViewColumn.Active,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")]
    }
  );
  let active: { id: string; controller: AbortController } | undefined;
  let completion: { id: string; controller: AbortController } | undefined;
  panel.onDidDispose(() => {
    active?.controller.abort();
    completion?.controller.abort();
  });
  panel.webview.html = html(context, panel.webview);
  panel.webview.onDidReceiveMessage(async (message) => {
    if (message?.type === "ready") {
      await panel.webview.postMessage({ type: "state", revision });
      return;
    }
    if (message?.type === "cancel") {
      const current = active;
      if (current === undefined) return;
      if (current.id === message.runId) {
        current.controller.abort();
        active = undefined;
        await panel.webview.postMessage({ type: "cancelled", runId: message.runId });
      }
      return;
    }
    if (message?.type === "cancelCompletion") {
      const current = completion;
      if (!current || current.id !== message.requestId) return;
      current.controller.abort();
      completion = undefined;
      await panel.webview.postMessage({
        type: "completionCancelled",
        requestId: current.id
      });
      return;
    }
    if (message?.type === "complete") {
      const requestId = typeof message.request?.id === "string"
        ? message.request.id
        : undefined;
      const command = message.request?.command;
      const term = validGraphCompletionTerm(message.request?.term);
      if (!requestId || !["ask", "explain", "cql"].includes(command) || !term) return;
      if (completion) {
        const previous = completion;
        previous.controller.abort();
        completion = undefined;
        await panel.webview.postMessage({
          type: "completionCancelled",
          requestId: previous.id
        });
      }
      const controller = new AbortController();
      completion = { id: requestId, controller };
      let timedOut = false;
      const timer = setTimeout(() => {
        timedOut = true;
        controller.abort();
      }, 4000);
      try {
        const args = buildCompletionArgs({
          term,
          graph: revision ? undefined : session.graphPath,
          revision
        });
        const result = await session.processes.run(
          session.root,
          args,
          controller.signal,
          { stdoutBytes: 1024 * 1024, stderrBytes: 64 * 1024 }
        );
        if (controller.signal.aborted) {
          if (timedOut) {
            await panel.webview.postMessage({
              type: "completionError",
              requestId,
              message: "Code graph search timed out."
            });
          }
          return;
        }
        if (result.code !== 0) {
          throw new QueryProcessError(result.stderr || result.stdout, result.code);
        }
        const response = CodeQueryResponseSchema.parse(JSON.parse(result.stdout));
        await panel.webview.postMessage({
          type: "completions",
          requestId,
          items: graphCompletionItems(response)
        });
      } catch (error) {
        if (controller.signal.aborted) return;
        console.error("[compass-query:completion] failed", error);
        await panel.webview.postMessage({
          type: "completionError",
          requestId,
          message: error instanceof QueryProcessError
            ? queryFailureMessage(error.output, error.exitCode)
            : "Compass returned graph suggestions this extension could not read."
        });
      } finally {
        clearTimeout(timer);
        if (completion?.controller === controller) completion = undefined;
      }
      return;
    }
    if (message?.type === "openSource") {
      const source = SourceLocationSchema.safeParse(message.source);
      if (!source.success) return;
      try {
        await openGraphSource(session, session.id, source.data);
      } catch (error) {
        void vscode.window.showErrorMessage(
          error instanceof Error ? error.message : String(error)
        );
      }
      return;
    }
    if (message?.type === "openGraph") {
      await vscode.commands.executeCommand("compass.openGraph", session.id);
      return;
    }
    if (message?.type !== "execute"
      || !["ask", "explain", "cql"].includes(message.request?.command)
      || typeof message.request.id !== "string"
      || typeof message.request.query !== "string") return;
    if (completion) {
      const previous = completion;
      previous.controller.abort();
      completion = undefined;
      await panel.webview.postMessage({
        type: "completionCancelled",
        requestId: previous.id
      });
    }
    if (active) {
      const previous = active;
      previous.controller.abort();
      await panel.webview.postMessage({ type: "cancelled", runId: previous.id });
    }
    const controller = new AbortController();
    const request = message.request as QuerySubmission & { id: string };
    const resolvedNodeId = validGraphCompletionNodeId(request.resolvedNodeId);
    active = { id: request.id, controller };
    const started = performance.now();
    try {
      const selection = {
        graph: revision ? undefined : session.graphPath,
        revision
      };
      const args = request.command === "cql"
        ? buildCqlArgs({
          ...request,
          ...selection
        })
        : request.command === "explain"
          ? buildExplainArgs({ query: resolvedNodeId ?? request.query, ...selection })
          : buildAskArgs({ query: request.query, ...selection });
      const result = await session.processes.run(session.root, args, controller.signal);
      if (controller.signal.aborted) return;
      if (result.code !== 0) {
        throw new QueryProcessError(result.stderr || result.stdout, result.code);
      }
      const output = request.command === "ask"
        ? { kind: "code-query" as const, value: CodeQueryResponseSchema.parse(JSON.parse(result.stdout)) }
        : request.command === "cql"
          ? { kind: "rows" as const, value: JSON.parse(result.stdout) as unknown }
          : { kind: "explanation" as const, text: result.stdout };
      await panel.webview.postMessage({
        type: "result",
        runId: request.id,
        revision,
        output,
        durationMs: Math.round(performance.now() - started)
      });
    } catch (error) {
      if (controller.signal.aborted) return;
      await panel.webview.postMessage({
        type: "error",
        runId: request.id,
        revision,
        message: error instanceof QueryProcessError
          ? queryFailureMessage(error.output, error.exitCode)
          : invalidResultMessage(request.command, error)
      });
    } finally {
      if (active?.controller === controller) active = undefined;
    }
  });
}

class QueryProcessError extends Error {
  constructor(
    public readonly output: string,
    public readonly exitCode: number
  ) {
    super(output);
  }
}

function invalidResultMessage(command: QuerySubmission["command"], error: unknown): string {
  console.error(`[compass-query:${command}] invalid result`, error);
  const label = command === "cql" ? "CompassQL" : command === "ask" ? "Ask" : "Explain";
  return `Compass returned a ${label} result this extension could not read. Update Compass and the extension, then try again.`;
}

function html(context: vscode.ExtensionContext, webview: vscode.Webview): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "query.js")
  );
  const styles = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "viewer.css")
  );
  const nonce = randomUUID().replaceAll("-", "");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<link rel="stylesheet" href="${styles}"><title>Compass Query</title></head>
<body><div id="root"></div><script nonce="${nonce}" src="${script}"></script></body></html>`;
}
