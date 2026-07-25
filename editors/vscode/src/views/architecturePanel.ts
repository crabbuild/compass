import { randomUUID } from "node:crypto";
import * as vscode from "vscode";
import { CallflowViewModelSchema } from "@compass/viewer/contracts/callflow";
import type { RepositorySession } from "../workspace/repositorySession";
import { openGraphSource } from "./sourceNavigation";

export async function openArchitecturePanel(
  context: vscode.ExtensionContext,
  session: RepositorySession,
  output: vscode.OutputChannel
): Promise<void> {
  const panel = vscode.window.createWebviewPanel(
    "compass.architecture",
    "Compass Architecture Flow",
    vscode.ViewColumn.Active,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")]
    }
  );
  const controller = new AbortController();
  let generation = 0;
  panel.onDidDispose(() => controller.abort());
  panel.webview.html = html(context, panel.webview);
  panel.webview.onDidReceiveMessage(async (message) => {
    if (message?.type === "ready" || message?.type === "retry") {
      const requestGeneration = ++generation;
      try {
        const model = await session.processes.runJson(
          session.root,
          ["export", "callflow-json", "--graph", session.graphPath],
          CallflowViewModelSchema,
          controller.signal
        );
        if (requestGeneration !== generation || controller.signal.aborted) return;
        await panel.webview.postMessage({
          type: "hydrate",
          repositoryId: session.id,
          model
        });
      } catch (error) {
        if (requestGeneration !== generation || controller.signal.aborted) return;
        const detail = error instanceof Error ? error.message : String(error);
        output.appendLine(`[error] Architecture export failed for ${session.root}: ${detail}`);
        await panel.webview.postMessage({
          type: "error",
          message: detail
        });
      }
    } else if (message?.type === "showOutput") {
      output.show(true);
    } else if (message?.type === "openSource" && typeof message.file === "string") {
      await openGraphSource(session, message.repositoryId, { file: message.file });
    }
  });
}

function html(context: vscode.ExtensionContext, webview: vscode.Webview): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "architecture.js")
  );
  const styles = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "viewer.css")
  );
  const nonce = randomUUID().replaceAll("-", "");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<link rel="stylesheet" href="${styles}"><title>Compass Architecture</title></head>
<body><div id="root"></div>
<script nonce="${nonce}" src="${script}"></script></body></html>`;
}
