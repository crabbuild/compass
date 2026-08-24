import path from "node:path";
import { randomUUID } from "node:crypto";
import { stat } from "node:fs/promises";
import * as vscode from "vscode";
import { z } from "zod";
import { buildInitArgs } from "../commands/buildArguments";
import { parseInitializationRequest } from "../initialize/panelMessages";
import { discoverScopeFiles } from "../initialize/scopeFiles";
import type { RepositorySession } from "../workspace/repositorySession";

const OCR_PROFILE = "pp-ocrv6-small";
const ModelsResponseSchema = z.object({
  schema: z.literal("compass.models/1"),
  profiles: z.array(z.object({
    profile: z.string().min(1).max(128),
    installed: z.boolean(),
    verified: z.boolean(),
    bytes: z.number().int().nonnegative().max(1_000_000_000),
    license: z.string().max(512)
  }).passthrough()).max(16)
}).strict();
type OcrModelStatus = import("@compass/viewer").OcrModelStatus;

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
  let ocrModel: OcrModelStatus = { kind: "checking", profile: OCR_PROFILE };
  let activeOcrModelInstall: ReturnType<RepositorySession["processes"]["startCommand"]> | undefined;
  const post = (message: unknown): Thenable<boolean> =>
    disposed ? Promise.resolve(false) : panel.webview.postMessage(message);
  const configPath = path.join(session.root, ".compass", "config.toml");
  let scopeFiles = discoverScopeFiles(session.root).catch((error) => {
    output.appendLine(
      `[init:scope] Could not enumerate repository files: ${firstLine(String(error))}`
    );
    return { files: [], truncated: false };
  });
  const hydrate = async () => {
    const discovered = await scopeFiles;
    return post({
      type: "hydrate",
      repositoryName: path.basename(session.root),
      repositoryRoot: session.root,
      configurationExists: await isFile(configPath),
      scopeFiles: discovered.files,
      scopeFilesTruncated: discovered.truncated,
      ocrModel
    });
  };

  const refreshOcrModel = async (): Promise<void> => {
    ocrModel = { kind: "checking", profile: OCR_PROFILE };
    await post({ type: "ocrModel", status: ocrModel });
    try {
      const response = await session.processes.runJson(
        session.root,
        ["models", "list", "--format", "json"],
        ModelsResponseSchema
      );
      const profile = response.profiles.find((candidate) => candidate.profile === OCR_PROFILE);
      if (!profile) {
        ocrModel = {
          kind: "error",
          profile: OCR_PROFILE,
          message: `The active Compass CLI does not advertise ${OCR_PROFILE}. Update Compass to enable managed OCR.`,
          canRetry: false
        };
      } else if (profile.verified) {
        ocrModel = {
          kind: "ready",
          profile: OCR_PROFILE,
          bytes: profile.bytes,
          engine: "OAR-OCR",
          engineVersion: "0.9.2"
        };
      } else if (profile.installed) {
        ocrModel = {
          kind: "invalid",
          profile: OCR_PROFILE,
          message: "The OCR model is present but its verification marker is stale or invalid.",
          installCommand: `compass models install ${OCR_PROFILE}`
        };
      } else {
        ocrModel = {
          kind: "missing",
          profile: OCR_PROFILE,
          installCommand: `compass models install ${OCR_PROFILE}`
        };
      }
    } catch (error) {
      ocrModel = {
        kind: "error",
        profile: OCR_PROFILE,
        message: firstLine(error instanceof Error ? error.message : String(error)),
        canRetry: true
      };
    }
    await post({ type: "ocrModel", status: ocrModel });
  };

  const installOcrModel = async (): Promise<void> => {
    if (activeOcrModelInstall || session.activeWriter) {
      ocrModel = {
        kind: "error",
        profile: OCR_PROFILE,
        message: "Finish the active Compass operation before installing the OCR model.",
        canRetry: true
      };
      await post({ type: "ocrModel", status: ocrModel });
      return;
    }
    ocrModel = { kind: "installing", profile: OCR_PROFILE };
    await post({ type: "ocrModel", status: ocrModel });
    const command = session.processes.startCommand(
      session.root,
      ["models", "install", OCR_PROFILE]
    );
    activeOcrModelInstall = command;
    session.activeWriter = command;
    try {
      output.appendLine(`> compass models install ${OCR_PROFILE}`);
      const result = await command.completed;
      output.append(result.stdout);
      output.append(result.stderr);
      if (result.code !== 0) {
        throw new Error(result.stderr || `Compass exited with ${result.code}`);
      }
      await refreshOcrModel();
    } catch (error) {
      ocrModel = {
        kind: "error",
        profile: OCR_PROFILE,
        message: firstLine(error instanceof Error ? error.message : String(error)),
        canRetry: true
      };
      await post({ type: "ocrModel", status: ocrModel });
    } finally {
      if (activeOcrModelInstall?.operationId === command.operationId) {
        activeOcrModelInstall = undefined;
      }
      if (session.activeWriter?.operationId === command.operationId) {
        session.activeWriter = undefined;
      }
    }
  };

  panel.onDidDispose(() => {
    disposed = true;
    activeOcrModelInstall?.cancel();
    activeOcrModelInstall = undefined;
  });
  panel.webview.html = html(context, panel.webview);
  panel.webview.onDidReceiveMessage(async (message) => {
    if (message?.type === "ready" || message?.type === "reset") {
      if (message.type === "reset") {
        scopeFiles = discoverScopeFiles(session.root).catch(() => ({
          files: [],
          truncated: false
        }));
      }
      await hydrate();
      void refreshOcrModel();
      return;
    }
    if (message?.type === "installOcrModel") {
      await installOcrModel();
      return;
    }
    if (message?.type === "verifyOcrModel") {
      await refreshOcrModel();
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
    if (session.activeWriter || activeOcrModelInstall) {
      await post({
        type: "failed",
        message: "Another Compass operation is already running."
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
  return (value.split(/\r?\n/, 1)[0] || "Compass initialization failed.").slice(0, 8_192);
}

async function isFile(target: string): Promise<boolean> {
  try {
    return (await stat(target)).isFile();
  } catch {
    return false;
  }
}
