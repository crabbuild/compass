import { randomUUID } from "node:crypto";
import path from "node:path";
import * as vscode from "vscode";
import { CallGraphResponseSchema } from "@compass/viewer/contracts/callGraph";
import type { CallDirection } from "@compass/viewer/contracts/callGraph";
import type { RepositorySession } from "../workspace/repositorySession";
import {
  callGraphCommandArguments,
  callGraphExpansionArguments,
  callGraphRootArguments
} from "./callGraphArguments";
import { utf8ByteAt } from "./cursorByte";
import { openGraphSource } from "./sourceNavigation";

export class CallGraphPanel {
  static async open(
    context: vscode.ExtensionContext,
    session: RepositorySession,
    editor: vscode.TextEditor,
    output: vscode.OutputChannel,
    initialDirection: CallDirection = "both"
  ): Promise<void> {
    const relative = path.relative(session.root, editor.document.uri.fsPath)
      .split(path.sep)
      .join("/");
    if (relative.startsWith("../")) {
      throw new Error("The active editor is outside the selected Compass repository.");
    }
    const byte = utf8ByteAt(editor.document, editor.selection.active);
    const line = editor.selection.active.line + 1;
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
    const panelController = new AbortController();
    let requestController: AbortController | undefined;
    let rootGeneration = 0;
    let direction = initialDirection;
    panel.onDidDispose(() => {
      requestController?.abort();
      panelController.abort();
    });
    panel.webview.html = html(context, panel.webview);
    panel.webview.onDidReceiveMessage(async (message) => {
      if (message?.type === "ready" || message?.type === "retry") {
        rootGeneration += 1;
        await send(
          callGraphRootArguments({ file: relative, byte, line }, direction, 2),
          "hydrateCallGraph",
          rootGeneration
        );
      } else if (message?.type === "changeDirection"
        && isDirection(message.direction)) {
        direction = message.direction;
        rootGeneration += 1;
        await send(
          callGraphRootArguments({ file: relative, byte, line }, direction, 2),
          "hydrateCallGraph",
          rootGeneration
        );
      } else if (message?.type === "showOutput") {
        output.show(true);
      } else if (message?.type === "expand"
        && typeof message.symbol === "string"
        && ["callers", "callees", "both"].includes(message.direction)
        && Number.isInteger(message.depth)) {
        await send(
          callGraphExpansionArguments(message.symbol, message.direction, message.depth),
          "mergeCallGraph",
          rootGeneration
        );
      } else if (message?.type === "openSource") {
        await openGraphSource(session, message.repositoryId, message.source);
      }
    });

    async function send(
      rootArgs: string[],
      type: "hydrateCallGraph" | "mergeCallGraph",
      generation: number
    ): Promise<void> {
      requestController?.abort();
      const controller = new AbortController();
      requestController = controller;
      const abort = () => controller.abort();
      panelController.signal.addEventListener("abort", abort, { once: true });
      try {
        const graphArgs = callGraphCommandArguments(rootArgs, session.graphPath);
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
        output.appendLine(`[error] Call graph failed for ${relative}:${line}: ${message}`);
        await panel.webview.postMessage({
          type: "error",
          message: userFacingError(message)
        });
      } finally {
        panelController.signal.removeEventListener("abort", abort);
        if (requestController === controller) requestController = undefined;
      }
    }
  }
}

function isDirection(value: unknown): value is CallDirection {
  return value === "callers" || value === "callees" || value === "both";
}

function userFacingError(message: string): string {
  return message.includes("no callable graph node matches")
    ? "Place the cursor inside a function or method included in the Compass graph."
    : message;
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
