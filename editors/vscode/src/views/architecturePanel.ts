import { randomUUID } from "node:crypto";
import { Buffer } from "node:buffer";
import * as vscode from "vscode";
import { CallflowViewModelSchema } from "@compass/viewer/contracts/callflow";
import type { ArchitectureEvidence, ArchitectureScope } from "@compass/viewer/contracts/architecture";
import type { RepositorySession } from "../workspace/repositorySession";
import { ArchitectureToHostMessageSchema } from "../transport/architectureMessages";
import { ArchitectureIndex } from "./architectureIndex";
import { openGraphSource } from "./sourceNavigation";

export const ARCHITECTURE_STDOUT_LIMIT = 128 * 1024 * 1024;

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
  const controller = new ArchitecturePanelController(context, session, panel, output);
  controller.initialize();
}

export class ArchitecturePanelController {
  private generation = 0;
  private disposed = false;
  private activeController: AbortController | undefined;
  private index: ArchitectureIndex | undefined;
  private scope: ArchitectureScope = "production";
  private evidence: ArchitectureEvidence = "all";

  constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly session: RepositorySession,
    private readonly panel: vscode.WebviewPanel,
    private readonly output: vscode.OutputChannel
  ) {}

  initialize(): void {
    this.panel.webview.html = html(this.context, this.panel.webview);
    this.panel.onDidDispose(() => this.dispose());
    this.panel.webview.onDidReceiveMessage((message) => {
      void this.handleMessage(message);
    });
  }

  async handleMessage(untrusted: unknown): Promise<void> {
    const parsed = ArchitectureToHostMessageSchema.safeParse(untrusted);
    if (!parsed.success || this.disposed) return;
    const message = parsed.data;
    if (message.type === "ready" || message.type === "retry") {
      await this.hydrate();
      return;
    }
    if (message.type === "showOutput") {
      this.output.show(true);
      return;
    }
    if (message.repositoryId !== this.session.id) return;
    if (message.type === "openSource") {
      await openGraphSource(this.session, message.repositoryId, { file: message.file });
      return;
    }
    if (message.type === "setArchitectureFilters") {
      this.scope = message.scope;
      this.evidence = message.evidence;
      await this.postOverview(message.requestId);
      return;
    }
    if (!this.index || message.generation !== this.generation) return;
    try {
      if (message.type === "requestSection") {
        const model = this.index.sectionPage({
          sectionId: message.sectionId,
          kind: message.kind,
          page: message.page,
          pageSize: message.pageSize,
          query: message.query,
          scope: message.scope,
          evidence: message.evidence
        });
        await this.panel.webview.postMessage({
          type: "architectureSectionPage",
          requestId: message.requestId,
          repositoryId: this.session.id,
          generation: this.generation,
          model
        });
      } else if (message.type === "requestRoute") {
        const model = this.index.routePage({
          routeId: message.routeId,
          page: message.page,
          pageSize: message.pageSize,
          query: message.query,
          scope: message.scope,
          evidence: message.evidence
        });
        await this.panel.webview.postMessage({
          type: "architectureRoutePage",
          requestId: message.requestId,
          repositoryId: this.session.id,
          generation: this.generation,
          model
        });
      } else if (message.type === "searchArchitecture") {
        const model = this.index.search({
          query: message.query,
          page: message.page,
          pageSize: message.pageSize,
          scope: message.scope,
          evidence: message.evidence
        });
        await this.panel.webview.postMessage({
          type: "architectureSearchResults",
          requestId: message.requestId,
          repositoryId: this.session.id,
          generation: this.generation,
          model
        });
      }
    } catch (error) {
      await this.postError(error);
    }
  }

  dispose(): void {
    this.disposed = true;
    this.generation += 1;
    this.activeController?.abort();
    this.activeController = undefined;
    this.index = undefined;
  }

  private async hydrate(): Promise<void> {
    this.activeController?.abort();
    const controller = new AbortController();
    this.activeController = controller;
    const generation = ++this.generation;
    this.index = undefined;
    this.scope = "production";
    this.evidence = "all";
    const started = Date.now();
    try {
      await this.postLoading("exporting", "Deriving complete architecture evidence");
      const result = await this.session.processes.run(
        this.session.root,
        ["export", "callflow-json", "--graph", this.session.graphPath],
        controller.signal,
        { stdoutBytes: ARCHITECTURE_STDOUT_LIMIT }
      );
      if (!this.isCurrent(generation, controller)) return;
      if (result.code !== 0) {
        throw new Error(result.stderr || `Compass exited with ${result.code}`);
      }
      const payloadBytes = Buffer.byteLength(result.stdout, "utf8");
      await this.postLoading("validating", `Validating ${formatBytes(payloadBytes)} export`);
      const model = CallflowViewModelSchema.parse(JSON.parse(result.stdout));
      if (!this.isCurrent(generation, controller)) return;
      await this.postLoading(
        "indexing",
        `Indexing ${model.statistics.edges.toLocaleString()} calls locally`
      );
      this.index = new ArchitectureIndex(model);
      await this.postLoading("mapping", "Laying out subsystem routes");
      await this.postOverview(randomUUID());
      this.output.appendLine(
        `[architecture] Loaded ${sessionLabel(this.session.root)}: `
        + `${model.statistics.nodes} symbols, ${model.statistics.edges} calls, `
        + `${formatBytes(payloadBytes)} in ${Date.now() - started} ms`
      );
    } catch (error) {
      if (!this.isCurrent(generation, controller)) return;
      this.output.appendLine(
        `[error] Architecture export failed for ${this.session.root}: ${errorText(error)}`
      );
      await this.postError(error);
    }
  }

  private async postOverview(requestId: string): Promise<void> {
    if (!this.index || this.disposed) return;
    await this.panel.webview.postMessage({
      type: "architectureOverview",
      requestId,
      repositoryId: this.session.id,
      generation: this.generation,
      model: this.index.overview(this.scope, this.evidence)
    });
  }

  private async postLoading(
    phase: "exporting" | "validating" | "indexing" | "mapping",
    message: string
  ): Promise<void> {
    await this.panel.webview.postMessage({ type: "architectureLoading", phase, message });
  }

  private async postError(error: unknown): Promise<void> {
    const detail = errorText(error);
    const message = detail.includes("128 MiB")
      ? `${detail}. The graph is valid, but this architecture document is larger than the local safety ceiling.`
      : detail;
    await this.panel.webview.postMessage({ type: "error", message, recoverable: true });
  }

  private isCurrent(generation: number, controller: AbortController): boolean {
    return !this.disposed
      && !controller.signal.aborted
      && generation === this.generation;
  }
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

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formatBytes(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(bytes >= 10 * 1024 * 1024 ? 1 : 2)} MiB`;
}

function sessionLabel(root: string): string {
  const normalized = root.replaceAll("\\", "/");
  return normalized.split("/").filter(Boolean).pop() ?? root;
}
