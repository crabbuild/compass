import { randomUUID } from "node:crypto";
import * as vscode from "vscode";
import {
  GraphViewModelSchema,
  type GraphViewModel
} from "@compass/viewer/contracts/graph";
import type { RepositorySession } from "../workspace/repositorySession";
import { GraphToHostMessageSchema } from "../transport/messages";
import { currentGraphExportArgs } from "./communityArguments";
import {
  graphOverviewCachePath,
  graphSourceInfo,
  loadCachedGraphOverview,
  loadPreparedGraphOverview,
  writeCachedGraphOverview
} from "./graphOverview";
import { CurrentGraphSnapshot } from "./graphSnapshot";
import { openGraphSource } from "./sourceNavigation";

const LARGE_GRAPH_BYTES = 8 * 1024 * 1024;

export class GraphPanel {
  static async open(
    context: vscode.ExtensionContext,
    session: RepositorySession,
    output: vscode.OutputChannel
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
    const graph = new GraphPanel(context, session, panel, output);
    await graph.initialize();
    return graph;
  }

  private readonly controller = new AbortController();
  private readonly communityCache = new Map<number, GraphViewModel>();
  private readonly snapshot = new CurrentGraphSnapshot();
  private snapshotReady: Promise<string> | undefined;
  private overview: GraphViewModel | undefined;

  private constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly session: RepositorySession,
    private readonly panel: vscode.WebviewPanel,
    private readonly output: vscode.OutputChannel
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
      if (parsed.data.type === "ready" || parsed.data.type === "retry") {
        await this.hydrate();
      } else if (parsed.data.type === "showOutput") {
        this.output.show(true);
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
      const nodeLimit = vscode.workspace
        .getConfiguration("compass")
        .get("graphNodeLimit", 5000);
      const source = await graphSourceInfo(this.session.graphPath);
      this.snapshotReady = this.snapshot.replace(this.session.graphPath);
      const storageRoot = this.context.storageUri ?? this.context.globalStorageUri;
      const cachePath = graphOverviewCachePath(storageRoot.fsPath, this.session.id);
      const model =
        await loadPreparedGraphOverview(this.session.graphPath, nodeLimit) ??
        await loadCachedGraphOverview(cachePath, this.session.graphPath, nodeLimit);

      if (model) {
        await this.publishOverview(model);
        return;
      }

      if (source.bytes >= LARGE_GRAPH_BYTES) {
        await this.postLoadStatus(source.bytes, "snapshotting");
      }
      const graphPath = await this.snapshotReady;
      if (source.bytes >= LARGE_GRAPH_BYTES) {
        await this.postLoadStatus(source.bytes, "exporting");
      }
      const exported = await this.session.processes.runJson(
        this.session.root,
        currentGraphExportArgs(graphPath, nodeLimit),
        GraphViewModelSchema,
        this.controller.signal
      );
      await this.publishOverview(exported);
      void writeCachedGraphOverview(
        cachePath,
        this.session.graphPath,
        nodeLimit,
        exported
      ).catch((error) => {
        this.output.appendLine(
          `[graph] Could not cache graph overview: ${
            error instanceof Error ? error.message : String(error)
          }`
        );
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
        const graphPath = await this.snapshotReady;
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

  private async publishOverview(model: GraphViewModel): Promise<void> {
    this.overview = this.withRepositoryTitle(model);
    this.communityCache.clear();
    await this.panel.webview.postMessage({
      type: "hydrateGraph",
      requestId: randomUUID(),
      repositoryId: this.session.id,
      model: this.overview
    });
  }

  private async postLoadStatus(
    graphBytes: number,
    phase: "snapshotting" | "exporting"
  ): Promise<void> {
    await this.panel.webview.postMessage({
      type: "graphLoadStatus",
      mode: "large",
      graphBytes,
      phase
    });
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
<div id="root"></div>
<script nonce="${nonce}" src="${script}"></script>
</body>
</html>`;
  }
}
