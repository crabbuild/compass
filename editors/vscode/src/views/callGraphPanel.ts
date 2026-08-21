import { randomUUID } from "node:crypto";
import path from "node:path";
import * as vscode from "vscode";
import type {
  CallDirection,
  CallGraphResponse
} from "@compass/viewer/contracts/callGraph";
import type { RepositorySession } from "../workspace/repositorySession";
import { runCallGraphAtCursor, runCallGraphForSymbol } from "./callGraphClient";
import { MAX_CALL_GRAPH_SYMBOL_LENGTH } from "./callGraphGuideMessages";
import { utf8ByteAt } from "./cursorByte";
import { openGraphSource } from "./sourceNavigation";

type CallGraphRoot = {
  kind: "cursor";
  relative: string;
  byte: number;
  line: number;
  title: string;
} | {
  kind: "symbol";
  symbol: string;
  title: string;
};

export class CallGraphPanel {
  static async open(
    context: vscode.ExtensionContext,
    session: RepositorySession,
    editor: vscode.TextEditor,
    output: vscode.OutputChannel,
    initialDirection: CallDirection = "both"
  ): Promise<void> {
    const relativePath = path.relative(session.root, editor.document.uri.fsPath);
    if (
      path.isAbsolute(relativePath)
      || relativePath === ".."
      || relativePath.startsWith(`..${path.sep}`)
    ) {
      throw new Error("The active editor is outside the selected Compass repository.");
    }
    const relative = relativePath.split(path.sep).join("/");
    const byte = utf8ByteAt(editor.document, editor.selection.active);
    const line = editor.selection.active.line + 1;
    openPanel(context, session, output, initialDirection, {
      kind: "cursor",
      relative,
      byte,
      line,
      title: path.basename(relative)
    });
  }

  static async openForSymbol(
    context: vscode.ExtensionContext,
    session: RepositorySession,
    symbol: string,
    output: vscode.OutputChannel,
    initialDirection: CallDirection = "both"
  ): Promise<void> {
    const normalized = symbol.trim();
    if (!normalized) throw new Error("Enter a function or method name.");
    if (normalized.length > MAX_CALL_GRAPH_SYMBOL_LENGTH) {
      throw new Error(
        `Symbol names must be ${MAX_CALL_GRAPH_SYMBOL_LENGTH} characters or fewer.`
      );
    }
    openPanel(context, session, output, initialDirection, {
      kind: "symbol",
      symbol: normalized,
      title: normalized
    });
  }
}

function openPanel(
  context: vscode.ExtensionContext,
  session: RepositorySession,
  output: vscode.OutputChannel,
  initialDirection: CallDirection,
  root: CallGraphRoot
): void {
  const panel = vscode.window.createWebviewPanel(
    "compass.callGraph",
    `Compass Calls — ${root.title}`,
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
        rootRequest(root, direction),
        "hydrateCallGraph",
        rootGeneration
      );
    } else if (message?.type === "changeDirection"
      && isDirection(message.direction)) {
      direction = message.direction;
      rootGeneration += 1;
      await send(
        rootRequest(root, direction),
        "hydrateCallGraph",
        rootGeneration
      );
    } else if (message?.type === "showOutput") {
      output.show(true);
    } else if (message?.type === "expand"
      && typeof message.symbol === "string"
      && isDirection(message.direction)
      && Number.isInteger(message.depth)) {
      await send(
        {
          kind: "symbol",
          symbol: message.symbol,
          direction: message.direction,
          depth: message.depth
        },
        "mergeCallGraph",
        rootGeneration
      );
    } else if (message?.type === "openSource") {
      await openGraphSource(session, message.repositoryId, message.source);
    }
  });

  async function send(
    request: {
      kind: "cursor";
      direction: CallDirection;
      depth: number;
    } | {
      kind: "symbol";
      symbol: string;
      direction: CallDirection;
      depth: number;
    },
    type: "hydrateCallGraph" | "mergeCallGraph",
    generation: number
  ): Promise<void> {
    requestController?.abort();
    const controller = new AbortController();
    requestController = controller;
    const abort = () => controller.abort();
    panelController.signal.addEventListener("abort", abort, { once: true });
    try {
      let graph: CallGraphResponse;
      if (request.kind === "cursor") {
        if (root.kind !== "cursor") {
          throw new Error("The call graph cursor root is unavailable.");
        }
        graph = await runCallGraphAtCursor(
          session,
          { file: root.relative, byte: root.byte, line: root.line },
          request.direction,
          request.depth,
          controller.signal
        );
      } else {
        graph = await runCallGraphForSymbol(
          session,
          request.symbol,
          request.direction,
          request.depth,
          controller.signal
        );
      }
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
      output.appendLine(`[error] Call graph failed for ${rootLabel(root)}: ${message}`);
      await panel.webview.postMessage({
        type: "error",
        message: userFacingError(message, request.kind)
      });
    } finally {
      panelController.signal.removeEventListener("abort", abort);
      if (requestController === controller) requestController = undefined;
    }
  }
}

function rootRequest(
  root: CallGraphRoot,
  direction: CallDirection
): {
  kind: "cursor";
  direction: CallDirection;
  depth: number;
} | {
  kind: "symbol";
  symbol: string;
  direction: CallDirection;
  depth: number;
} {
  return root.kind === "cursor"
    ? { kind: "cursor", direction, depth: 2 }
    : { kind: "symbol", symbol: root.symbol, direction, depth: 2 };
}

function rootLabel(root: CallGraphRoot): string {
  return root.kind === "cursor"
    ? `${root.relative}:${root.line}`
    : `symbol ${JSON.stringify(root.symbol)}`;
}

function isDirection(value: unknown): value is CallDirection {
  return value === "callers" || value === "callees" || value === "both";
}

function userFacingError(message: string, rootKind: "cursor" | "symbol"): string {
  if (!message.includes("no callable graph node matches")) return message;
  return rootKind === "cursor"
    ? "Place the cursor inside a function or method included in the Compass graph."
    : "No callable function or method matched that symbol. Try its qualified name.";
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
