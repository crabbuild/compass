import path from "node:path";
import { randomUUID } from "node:crypto";
import { stat } from "node:fs/promises";
import * as vscode from "vscode";
import { buildInitArgs } from "../commands/buildArguments";
import { parseInitializationRequest } from "../initialize/panelMessages";
import type { RepositorySession } from "../workspace/repositorySession";

export async function openInitializationPanel(
  context: vscode.ExtensionContext,
  session: RepositorySession,
  output: vscode.OutputChannel,
  refresh: () => Promise<void>
): Promise<void> {
  const panel = vscode.window.createWebviewPanel(
    "compass.initialize",
    `Initialize Compass — ${path.basename(session.root)}`,
    vscode.ViewColumn.Active,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")]
    }
  );
  let disposed = false;
  let activeOperation:
    | {
      operationId: string;
      cancel(): void;
      cancelRequested: boolean;
    }
    | undefined;
  const post = (message: unknown): Thenable<boolean> =>
    disposed ? Promise.resolve(false) : panel.webview.postMessage(message);
  const configPath = path.join(session.root, ".compass", "config.toml");
  const hydrate = async () => post({
    type: "hydrate",
    repositoryName: path.basename(session.root),
    repositoryRoot: session.root,
    configurationExists: await isFile(configPath)
  });

  panel.onDidDispose(() => {
    disposed = true;
  });
  panel.webview.html = html(context, panel.webview);
  panel.webview.onDidReceiveMessage(async (message) => {
    if (message?.type === "ready" || message?.type === "reset") {
      await hydrate();
      return;
    }
    if (message?.type === "showOutput") {
      output.show(true);
      return;
    }
    if (message?.type === "openGraph") {
      await vscode.commands.executeCommand("compass.openGraph", session.id);
      return;
    }
    if (message?.type === "cancel") {
      if (activeOperation) {
        activeOperation.cancelRequested = true;
        activeOperation.cancel();
      }
      return;
    }
    const request = parseInitializationRequest(message);
    if (!request) return;
    if (session.activeWriter) {
      await post({
        type: "failed",
        message: "Another Compass write operation is already running."
      });
      return;
    }

    const configurationExisted = await isFile(configPath);
    if (
      configurationExisted
      && !request.replaceExisting
    ) {
      await post({
        type: "failed",
        message: "An existing Compass configuration must be reviewed before it can be replaced."
      });
      return;
    }
    const args = buildInitArgs({
      root: session.root,
      includes: request.includes,
      excludes: request.excludes,
      force: configurationExisted
    });
    output.appendLine(`> compass ${args.join(" ")}`);
    const command = session.processes.startJsonl(
      session.root,
      args,
      (event) => {
        output.appendLine(`[${event.phase}] ${event.message}`);
        void post({ type: "progress", event });
      }
    );
    const operation = {
      operationId: command.operationId,
      cancel: command.cancel,
      cancelRequested: false
    };
    activeOperation = operation;
    session.activeWriter = command;
    session.graphState = "building";
    await refresh();

    try {
      const result = await command.completed;
      output.append(result.stdout);
      output.append(result.stderr);
      if (result.code !== 0) {
        throw new Error(result.stderr || `Compass exited with ${result.code}`);
      }
      if (activeOperation?.operationId !== operation.operationId) return;
      session.graphState = "available";
      await post({
        type: "succeeded",
        message: `${path.basename(session.root)} is indexed and ready for graph exploration.`
      });
    } catch (error) {
      if (activeOperation?.operationId !== operation.operationId) return;
      await post({
        type: "configurationChanged",
        configurationExists: await isFile(configPath)
      });
      if (operation.cancelRequested) {
        session.graphState = "not-materialized";
        await post({ type: "cancelled" });
      } else {
        session.graphState = "failed";
        const detail = error instanceof Error ? error.message : String(error);
        output.appendLine(`[init:error] ${detail}`);
        await post({ type: "failed", message: firstLine(detail) });
      }
    } finally {
      if (session.activeWriter?.operationId === operation.operationId) {
        session.activeWriter = undefined;
      }
      if (activeOperation?.operationId === operation.operationId) {
        activeOperation = undefined;
      }
      await refresh();
    }
  });
}

function html(context: vscode.ExtensionContext, webview: vscode.Webview): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "initialize.js")
  );
  const styles = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "viewer.css")
  );
  const nonce = randomUUID().replaceAll("-", "");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<link rel="stylesheet" href="${styles}"><title>Initialize Compass</title></head>
<body><div id="root"></div><script nonce="${nonce}" src="${script}"></script></body></html>`;
}

function firstLine(value: string): string {
  return value.split(/\r?\n/, 1)[0] || "Compass initialization failed.";
}

async function isFile(target: string): Promise<boolean> {
  try {
    return (await stat(target)).isFile();
  } catch {
    return false;
  }
}
