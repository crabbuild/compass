import { randomUUID } from "node:crypto";
import * as vscode from "vscode";
import type { CallDirection } from "@compass/viewer/contracts/callGraph";
import { CodeQueryResponseSchema } from "@compass/viewer";
import { buildCompletionArgs } from "../commands/queryArguments";
import type { RepositorySession } from "../workspace/repositorySession";
import {
  parseCallGraphCompletionRequest,
  parseCallGraphSymbolRequest
} from "./callGraphGuideMessages";
import { queryFailureMessage } from "./queryExecution";
import { callGraphCompletionItems } from "./queryCompletions";

type CallGraphSource = {
  uri: vscode.Uri;
  selection: vscode.Selection;
  viewColumn: vscode.ViewColumn | undefined;
  fileLabel: string;
  languageId: string;
};

const directionCommands: Record<CallDirection, string> = {
  callers: "compass.openCallers",
  callees: "compass.openCallees",
  both: "compass.openCallersAndCallees"
};

export function openCallGraphGuidePanel(
  context: vscode.ExtensionContext,
  editor: vscode.TextEditor | undefined,
  session: RepositorySession,
  openSymbol: (symbol: string, direction: CallDirection) => Promise<boolean>
): void {
  const source = editor ? captureSource(editor) : undefined;
  const panel = vscode.window.createWebviewPanel(
    "compass.callGraphGuide",
    "Compass Call Graph",
    vscode.ViewColumn.Active,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")]
    }
  );
  let completion: { id: string; controller: AbortController } | undefined;
  panel.onDidDispose(() => completion?.controller.abort());
  panel.webview.html = html(context, panel.webview);
  panel.webview.onDidReceiveMessage(async (message) => {
    if (message?.type === "ready") {
      await panel.webview.postMessage({
        type: "hydrate",
        source: source
          ? { fileLabel: source.fileLabel, languageId: source.languageId }
          : null
      });
      return;
    }
    if (message?.type === "cancelCompletion") {
      const requestId = typeof message.requestId === "string"
        && message.requestId.length <= 128
        ? message.requestId
        : undefined;
      const current = completion;
      if (!requestId || !current || current.id !== requestId) return;
      current.controller.abort();
      completion = undefined;
      await panel.webview.postMessage({
        type: "completionCancelled",
        requestId
      });
      return;
    }
    const completionRequest = parseCallGraphCompletionRequest(message);
    if (completionRequest) {
      if (!vscode.workspace.isTrusted) {
        await panel.webview.postMessage({
          type: "completionError",
          requestId: completionRequest.requestId,
          message: "Trust this workspace to search its Compass code graph."
        });
        return;
      }
      if (session.capabilityError) {
        await panel.webview.postMessage({
          type: "completionError",
          requestId: completionRequest.requestId,
          message: "Upgrade or select a compatible Compass CLI to search graph symbols."
        });
        return;
      }
      if (session.graphState !== "available") {
        await panel.webview.postMessage({
          type: "completionError",
          requestId: completionRequest.requestId,
          message: "Build the Compass code graph to search callable symbols."
        });
        return;
      }
      if (completion) {
        const previous = completion;
        previous.controller.abort();
        completion = undefined;
        await panel.webview.postMessage({
          type: "completionCancelled",
          requestId: previous.id
        });
      }
      const controller = new AbortController();
      completion = { id: completionRequest.requestId, controller };
      let timedOut = false;
      const timer = setTimeout(() => {
        timedOut = true;
        controller.abort();
      }, 4000);
      try {
        const result = await session.processes.run(
          session.root,
          buildCompletionArgs({
            term: completionRequest.term,
            graph: session.graphPath
          }),
          controller.signal,
          { stdoutBytes: 1024 * 1024, stderrBytes: 64 * 1024 }
        );
        if (controller.signal.aborted) {
          if (timedOut) {
            await panel.webview.postMessage({
              type: "completionError",
              requestId: completionRequest.requestId,
              message: "Code graph search timed out."
            });
          }
          return;
        }
        if (result.code !== 0) {
          throw new CompletionProcessError(
            result.stderr || result.stdout,
            result.code
          );
        }
        const response = CodeQueryResponseSchema.parse(JSON.parse(result.stdout));
        await panel.webview.postMessage({
          type: "completions",
          requestId: completionRequest.requestId,
          items: callGraphCompletionItems(response)
        });
      } catch (error) {
        if (controller.signal.aborted) return;
        console.error("[compass-call-guide:completion] failed", error);
        await panel.webview.postMessage({
          type: "completionError",
          requestId: completionRequest.requestId,
          message: error instanceof CompletionProcessError
            ? queryFailureMessage(error.output, error.exitCode)
            : "Compass returned graph suggestions this extension could not read."
        });
      } finally {
        clearTimeout(timer);
        if (completion?.controller === controller) completion = undefined;
      }
      return;
    }
    if (message?.type === "openWalkthrough") {
      await vscode.commands.executeCommand(
        "workbench.action.openWalkthrough",
        "crabbuild.crabbuild-compass-vscode#compass.getStarted",
        false
      );
      return;
    }
    const symbolRequest = parseCallGraphSymbolRequest(message);
    if (symbolRequest) {
      completion?.controller.abort();
      completion = undefined;
      const opened = await openSymbol(
        symbolRequest.symbol,
        symbolRequest.direction
      );
      if (opened) {
        panel.dispose();
      } else {
        await panel.webview.postMessage({ type: "openSymbolFailed" });
      }
      return;
    }
    if (message?.type === "openSymbol") {
      await panel.webview.postMessage({ type: "openSymbolFailed" });
      return;
    }
    if (message?.type !== "openDirection") {
      return;
    }
    const candidate: unknown = message.direction;
    if (!isDirection(candidate)) return;
    const direction: CallDirection = candidate;
    if (!source) {
      void vscode.window.showInformationMessage(
        "Open an indexed source file, place the cursor inside a function, then reopen the call graph guide."
      );
      return;
    }
    panel.dispose();
    try {
      const document = await vscode.workspace.openTextDocument(source.uri);
      const options: vscode.TextDocumentShowOptions = {
        preserveFocus: false,
        preview: false,
        selection: source.selection
      };
      if (source.viewColumn !== undefined) {
        options.viewColumn = source.viewColumn;
      }
      await vscode.window.showTextDocument(document, options);
      await vscode.commands.executeCommand(directionCommands[direction]);
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      void vscode.window.showErrorMessage(
        `Compass could not restore the source editor: ${detail}`
      );
    }
  });
}

class CompletionProcessError extends Error {
  constructor(
    readonly output: string,
    readonly exitCode: number
  ) {
    super(output);
  }
}

function captureSource(editor: vscode.TextEditor): CallGraphSource {
  const relative = vscode.workspace.asRelativePath(editor.document.uri, false);
  return {
    uri: editor.document.uri,
    selection: editor.selection,
    viewColumn: editor.viewColumn,
    fileLabel: relative || editor.document.fileName,
    languageId: editor.document.languageId
  };
}

function isDirection(value: unknown): value is CallDirection {
  return value === "callers" || value === "callees" || value === "both";
}

function html(context: vscode.ExtensionContext, webview: vscode.Webview): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "callGraphGuide.js")
  );
  const styles = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "viewer.css")
  );
  const nonce = randomUUID().replaceAll("-", "");
  return `<!doctype html><html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<link rel="stylesheet" href="${styles}"><title>Compass Call Graph Guide</title>
</head><body><div id="root"></div>
<script nonce="${nonce}" src="${script}"></script></body></html>`;
}
