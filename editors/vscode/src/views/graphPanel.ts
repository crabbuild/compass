import { randomUUID } from "node:crypto";
import * as vscode from "vscode";
import {
  GraphViewModelSchema,
  type GraphViewModel
} from "@compass/viewer/contracts/graph";
import type { RepositorySession } from "../workspace/repositorySession";
import { GraphToHostMessageSchema } from "../transport/messages";
import { currentGraphExportArgs } from "./communityArguments";
import { CurrentGraphSnapshot } from "./graphSnapshot";
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
  private readonly communityCache = new Map<number, GraphViewModel>();
  private readonly snapshot = new CurrentGraphSnapshot();
  private overview: GraphViewModel | undefined;

  private constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly session: RepositorySession,
    private readonly panel: vscode.WebviewPanel
  ) {}

  private async initialize(): Promise<void> {
    this.panel.webview.html = this.html();
    this.panel.onDidDispose(() => {
      this.controller.abort();
      void this.snapshot.dispose();
    });
    this.panel.webview.onDidReceiveMessage(async (untrusted) => {
      const parsed = GraphToHostMessageSchema.safeParse(untrusted);
      if (!parsed.success) return;
      if (parsed.data.type === "ready") {
        await this.hydrate();
      } else if (parsed.data.type === "openSource") {
        try {
          await openGraphSource(this.session, parsed.data.repositoryId, parsed.data.source);
        } catch (error) {
          void vscode.window.showErrorMessage(
            error instanceof Error ? error.message : String(error)
          );
        }
      } else {
        await this.openCommunity(
          parsed.data.requestId,
          parsed.data.repositoryId,
          parsed.data.communityId
        );
      }
    });
  }

  private async hydrate(): Promise<void> {
    try {
      const graphPath = await this.snapshot.replace(this.session.graphPath);
      const model = await this.session.processes.runJson(
        this.session.root,
        currentGraphExportArgs(
          graphPath,
          vscode.workspace.getConfiguration("compass").get("graphNodeLimit", 5000)
        ),
        GraphViewModelSchema,
        this.controller.signal
      );
      this.overview = this.withRepositoryTitle(model);
      this.communityCache.clear();
      await this.panel.webview.postMessage({
        type: "hydrateGraph",
        requestId: randomUUID(),
        repositoryId: this.session.id,
        model: this.overview
      });
    } catch (error) {
      await this.panel.webview.postMessage({
        type: "error",
        message: error instanceof Error ? error.message : String(error)
      });
    }
  }

  private async openCommunity(
    requestId: string,
    repositoryId: string,
    communityId: number
  ): Promise<void> {
    try {
      if (repositoryId !== this.session.id) {
        throw new Error("This community request belongs to another repository.");
      }
      if (this.session.capabilities?.features.community_detail !== true) {
        throw new Error(
          "The installed Compass CLI does not support lazy community details. Upgrade Compass and reload VS Code."
        );
      }
      const summary = this.overview?.nodes.find(
        (node) => node.community === communityId && node.memberCount !== undefined
      );
      if (!this.overview?.stats.aggregated || !summary) {
        throw new Error(`Community ${communityId} is not present in the current overview.`);
      }
      let model = this.communityCache.get(communityId);
      if (!model) {
        const graphPath = this.snapshot.graphPath;
        if (!graphPath) throw new Error("The graph snapshot is no longer available.");
        model = await this.session.processes.runJson(
          this.session.root,
          currentGraphExportArgs(
            graphPath,
            vscode.workspace.getConfiguration("compass").get("graphNodeLimit", 5000),
            communityId
          ),
          GraphViewModelSchema,
          this.controller.signal
        );
        model = this.withRepositoryTitle(model);
        this.communityCache.set(communityId, model);
        if (this.communityCache.size > 3) {
          const oldest = this.communityCache.keys().next().value;
          if (oldest !== undefined) this.communityCache.delete(oldest);
        }
      }
      await this.panel.webview.postMessage({
        type: "communityGraph",
        requestId,
        repositoryId: this.session.id,
        communityId,
        model
      });
    } catch (error) {
      await this.panel.webview.postMessage({
        type: "communityError",
        requestId,
        communityId,
        message: error instanceof Error ? error.message : String(error)
      });
    }
  }

  private withRepositoryTitle(model: GraphViewModel): GraphViewModel {
    return {
      ...model,
      title: vscode.workspace.asRelativePath(this.session.graphPath)
    };
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
