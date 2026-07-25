import { randomUUID } from "node:crypto";
import * as vscode from "vscode";
import { SourceLocationSchema } from "@compass/viewer/contracts/graph";
import { buildCqlArgs, buildNaturalQueryArgs } from "../commands/queryArguments";
import type { RepositorySession } from "../workspace/repositorySession";
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
  let active: AbortController | undefined;
  panel.onDidDispose(() => active?.abort());
  panel.webview.html = html(context, panel.webview);
  panel.webview.onDidReceiveMessage(async (message) => {
    if (message?.type === "ready") {
      await panel.webview.postMessage({ type: "state", running: false, revision });
      return;
    }
    if (message?.type === "cancel") {
      active?.abort();
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
      || !["natural", "cql"].includes(message.request?.mode)
      || typeof message.request.query !== "string") return;
    active?.abort();
    const controller = new AbortController();
    active = controller;
    const started = performance.now();
    await panel.webview.postMessage({ type: "state", running: true, revision });
    try {
      const request = message.request as {
        mode: "natural" | "cql";
        query: string;
        params: Record<string, string>;
        timeoutMs: number;
        maxRows: number;
      };
      const args = request.mode === "cql"
        ? buildCqlArgs({
          ...request,
          graph: revision ? undefined : session.graphPath,
          revision
        })
        : buildNaturalQueryArgs({
          query: request.query,
          graph: revision ? undefined : session.graphPath,
          revision
        });
      const result = await session.processes.run(session.root, args, controller.signal);
      if (result.code !== 0) throw new Error(result.stderr || `Compass exited with ${result.code}`);
      await panel.webview.postMessage({
        type: "result",
        revision,
        result: {
          mode: request.mode,
          ...(request.mode === "cql"
            ? { json: JSON.parse(result.stdout) }
            : { text: result.stdout }),
          durationMs: Math.round(performance.now() - started)
        }
      });
    } catch (error) {
      await panel.webview.postMessage({
        type: "error",
        revision,
        message: error instanceof Error ? error.message : String(error)
      });
    } finally {
      if (active === controller) active = undefined;
    }
  });
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
