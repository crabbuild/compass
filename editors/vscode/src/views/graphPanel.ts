import { randomUUID } from "node:crypto";
import * as vscode from "vscode";
import { GraphViewModelSchema } from "@compass/viewer/contracts/graph";
import type { RepositorySession } from "../workspace/repositorySession";
import { GraphToHostMessageSchema } from "../transport/messages";
import { openGraphSource } from "./sourceNavigation";

export class GraphPanel {
  static async open(
    context: vscode.ExtensionContext,
    session: RepositorySession
  ): Promise<GraphPanel> {
    const panel = vscode.window.createWebviewPanel(
      "compass.graph",
      `Compass Graph — ${vscode.workspace.asRelativePath(session.root)}`,
      vscode.ViewColumn.Active,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")]
      }
    );
    const graph = new GraphPanel(context, session, panel);
    await graph.initialize();
    return graph;
  }

  private readonly controller = new AbortController();

  private constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly session: RepositorySession,
    private readonly panel: vscode.WebviewPanel
  ) {}

  private async initialize(): Promise<void> {
    this.panel.webview.html = this.html();
    this.panel.onDidDispose(() => this.controller.abort());
    this.panel.webview.onDidReceiveMessage(async (untrusted) => {
      const parsed = GraphToHostMessageSchema.safeParse(untrusted);
      if (!parsed.success) return;
      if (parsed.data.type === "ready") {
        await this.hydrate();
      } else {
        try {
          await openGraphSource(this.session, parsed.data.repositoryId, parsed.data.source);
        } catch (error) {
          void vscode.window.showErrorMessage(
            error instanceof Error ? error.message : String(error)
          );
        }
      }
    });
  }

  private async hydrate(): Promise<void> {
    try {
      const model = await this.session.processes.runJson(
        this.session.root,
        [
          "export",
          "viewer-json",
          "--graph",
          this.session.graphPath,
          "--node-limit",
          String(vscode.workspace.getConfiguration("compass").get("graphNodeLimit", 5000))
        ],
        GraphViewModelSchema,
        this.controller.signal
      );
      await this.panel.webview.postMessage({
        type: "hydrateGraph",
        requestId: randomUUID(),
        repositoryId: this.session.id,
        model
      });
    } catch (error) {
      await this.panel.webview.postMessage({
        type: "error",
        message: error instanceof Error ? error.message : String(error)
      });
    }
  }

  private html(): string {
    const webview = this.panel.webview;
    const script = webview.asWebviewUri(
      vscode.Uri.joinPath(this.context.extensionUri, "dist", "webviews", "graph.js")
    );
    const styles = webview.asWebviewUri(
      vscode.Uri.joinPath(this.context.extensionUri, "dist", "webviews", "viewer.css")
    );
    const nonce = randomUUID().replaceAll("-", "");
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${webview.cspSource} data:; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<link rel="stylesheet" href="${styles}">
<title>Compass Code Graph</title>
</head>
<body>
<div id="root" role="status" aria-live="polite">Loading Compass graph…</div>
<script nonce="${nonce}" src="${script}"></script>
</body>
</html>`;
  }
}
