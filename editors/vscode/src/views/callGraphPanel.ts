import { randomUUID } from "node:crypto";
import path from "node:path";
import * as vscode from "vscode";
import { CallGraphResponseSchema } from "@compass/viewer/contracts/callGraph";
import type { RepositorySession } from "../workspace/repositorySession";
import { utf8ByteAt } from "./cursorByte";
import { openGraphSource } from "./sourceNavigation";

export class CallGraphPanel {
  static async open(
    context: vscode.ExtensionContext,
    session: RepositorySession,
    editor: vscode.TextEditor,
    output: vscode.OutputChannel
  ): Promise<void> {
    const relative = path.relative(session.root, editor.document.uri.fsPath)
      .split(path.sep)
      .join("/");
    if (relative.startsWith("../")) {
      throw new Error("The active editor is outside the selected Compass repository.");
    }
    const byte = utf8ByteAt(editor.document, editor.selection.active);
    const panel = vscode.window.createWebviewPanel(
      "compass.callGraph",
      `Compass Calls — ${path.basename(relative)}`,
      vscode.ViewColumn.Active,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")]
      }
    );
    const controller = new AbortController();
    let rootGeneration = 0;
    panel.onDidDispose(() => controller.abort());
    panel.webview.html = html(context, panel.webview);
    panel.webview.onDidReceiveMessage(async (message) => {
      if (message?.type === "ready" || message?.type === "retry") {
        rootGeneration += 1;
        await send([
          "--at", `${relative}:${byte}`,
          "--direction", "both",
          "--depth", "2"
        ], "hydrateCallGraph", rootGeneration);
      } else if (message?.type === "showOutput") {
        output.show(true);
      } else if (message?.type === "expand"
        && typeof message.symbol === "string"
        && ["callers", "callees", "both"].includes(message.direction)
        && Number.isInteger(message.depth)) {
        await send([
          "--symbol", message.symbol,
          "--direction", message.direction,
          "--depth", String(message.depth)
        ], "mergeCallGraph", rootGeneration);
      } else if (message?.type === "openSource") {
        await openGraphSource(session, message.repositoryId, message.source);
      }
    });

    async function send(
      rootArgs: string[],
      type: "hydrateCallGraph" | "mergeCallGraph",
      generation: number
    ): Promise<void> {
      try {
        const graphArgs = [
          "program", "call-graph",
          ...rootArgs,
          "--max-nodes", "500",
          "--max-edges", "1000",
          "--program", session.programPath,
          "--graph", session.graphPath,
          "--format", "json"
        ];
        const graph = await session.processes.runJson(
          session.root,
          graphArgs,
          CallGraphResponseSchema,
          controller.signal
        );
        if (generation !== rootGeneration || controller.signal.aborted) return;
        await panel.webview.postMessage({
          type,
          requestId: randomUUID(),
          repositoryId: session.id,
          graph
        });
      } catch (error) {
        if (generation !== rootGeneration || controller.signal.aborted) return;
        const message = error instanceof Error ? error.message : String(error);
        output.appendLine(`[error] Call graph failed for ${relative}:${byte}: ${message}`);
        await panel.webview.postMessage({
          type: "error",
          message
        });
      }
    }
  }
}

function html(context: vscode.ExtensionContext, webview: vscode.Webview): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "callGraph.js")
  );
  const styles = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "viewer.css")
  );
  const nonce = randomUUID().replaceAll("-", "");
  return `<!doctype html><html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<link rel="stylesheet" href="${styles}"><title>Compass Call Graph</title>
</head><body><div id="root"></div>
<script nonce="${nonce}" src="${script}"></script></body></html>`;
}
