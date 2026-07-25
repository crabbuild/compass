import { createHash, randomUUID } from "node:crypto";
import * as vscode from "vscode";
import { buildHistoryArgs } from "../history/buildArguments";
import { loadSemanticDiff } from "../history/diffClient";
import { loadChangeCounts } from "../history/changeCountsClient";
import { RevisionStore } from "../history/revisionStore";
import { loadTimeline } from "../history/timelineClient";
import type { RepositorySession } from "../workspace/repositorySession";
import { openGraphSource } from "./sourceNavigation";
import { openQueryPanel } from "./queryPanel";

export async function openHistoryPanel(
  context: vscode.ExtensionContext,
  session: RepositorySession,
  output: vscode.OutputChannel
): Promise<void> {
  const panel = vscode.window.createWebviewPanel(
    "compass.history",
    "Compass Codebase Evolution",
    vscode.ViewColumn.Active,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")]
    }
  );
  const storageRoot = context.storageUri ?? context.globalStorageUri;
  const repositoryKey = createHash("sha256").update(session.root).digest("hex").slice(0, 20);
  const revisions = new RevisionStore(
    vscode.Uri.joinPath(storageRoot, "history", repositoryKey).fsPath,
    session
  );
  await revisions.initialize();
  let timeline = await loadTimeline(session);
  const countCache = new Map<string, Awaited<ReturnType<typeof loadChangeCounts>>>();
  panel.webview.html = html(context, panel.webview);
  panel.webview.onDidReceiveMessage(async (message) => {
    try {
      if (message?.type === "ready") {
        await panel.webview.postMessage({ type: "timeline", timeline, repositoryId: session.id });
      } else if (message?.type === "loadRevision" && typeof message.commit === "string") {
        const revision = await revisions.load(message.commit);
        await panel.webview.postMessage({
          type: "graph",
          commit: message.commit,
          graph: revision.graph
        });
      } else if (message?.type === "buildRevision" && typeof message.commit === "string") {
        if (session.activeWriter) {
          throw new Error("Another Compass write operation is already running.");
        }
        const profile = await vscode.window.showQuickPick(
          [
            {
              label: "Configured history profile",
              description: "Use the repository's enabled Compass history profile",
              value: { kind: "configured" as const }
            },
            {
              label: "Code only",
              description: "Build locally from AST and inferred evidence without model credentials",
              value: { kind: "code-only" as const }
            },
            {
              label: "Reuse a profile",
              description: "Copy the build profile from another revision or realization",
              value: { kind: "from" as const }
            }
          ],
          { title: `Build graph for ${message.commit.slice(0, 9)}` }
        );
        if (!profile) return;
        let selectedProfile:
          | { kind: "configured" | "code-only" }
          | { kind: "from"; source: string };
        if (profile.value.kind === "from") {
          const source = await vscode.window.showInputBox({
            title: "Reuse Compass history profile",
            prompt: "Enter a revision or realization ID"
          });
          if (!source) return;
          selectedProfile = { kind: "from", source };
        } else {
          selectedProfile = profile.value;
        }
        const command = session.processes.startJsonl(
          session.root,
          buildHistoryArgs({
            revision: message.commit,
            all: false,
            firstParent: false,
            profile: selectedProfile
          }),
          (event) => output.appendLine(`[history:${event.phase}] ${event.message}`)
        );
        session.activeWriter = command;
        let result: Awaited<typeof command.completed>;
        try {
          result = await vscode.window.withProgress(
            {
              location: vscode.ProgressLocation.Notification,
              title: `Building Compass graph for ${message.commit.slice(0, 9)}`,
              cancellable: true
            },
            async (_, token) => {
              token.onCancellationRequested(() => command.cancel());
              return command.completed;
            }
          );
        } finally {
          if (session.activeWriter?.operationId === command.operationId) {
            session.activeWriter = undefined;
          }
        }
        output.append(result.stdout);
        output.append(result.stderr);
        if (result.code !== 0) throw new Error(result.stderr || `Compass exited with ${result.code}`);
        timeline = await loadTimeline(session);
        await panel.webview.postMessage({ type: "timeline", timeline, repositoryId: session.id });
      } else if (message?.type === "compare"
        && typeof message.commit === "string"
        && typeof message.parent === "string") {
        const currentEntry = timeline.entries.find((entry) => entry.commit === message.commit);
        const parentEntry = timeline.entries.find((entry) => entry.commit === message.parent);
        if (!currentEntry?.presentationAvailable || !parentEntry?.presentationAvailable) {
          throw new Error("Both revisions must have graph available before comparison.");
        }
        const [current, parent, semanticDiff] = await Promise.all([
          revisions.load(message.commit),
          revisions.load(message.parent),
          loadSemanticDiff(session, message.parent, message.commit)
        ]);
        await panel.webview.postMessage({
          type: "comparison",
          commit: message.commit,
          parent: message.parent,
          currentGraph: current.graph,
          parentGraph: parent.graph,
          semanticDiff
        });
      } else if (message?.type === "queryRevision" && typeof message.commit === "string") {
        const entry = timeline.entries.find((candidate) => candidate.commit === message.commit);
        if (!entry?.presentationAvailable) {
          throw new Error("Build this revision's graph before querying it.");
        }
        await openQueryPanel(context, session, message.commit);
      } else if (message?.type === "changeCounts" && typeof message.commit === "string") {
        let counts = countCache.get(message.commit);
        if (!counts) {
          counts = await loadChangeCounts(session, message.commit);
          countCache.set(message.commit, counts);
        }
        await panel.webview.postMessage({ type: "changeCounts", counts });
      } else if (message?.type === "openSource") {
        await openGraphSource(session, message.repositoryId, message.source);
      }
    } catch (error) {
      await panel.webview.postMessage({
        type: "error",
        message: error instanceof Error ? error.message : String(error)
      });
    }
  });
}

function html(context: vscode.ExtensionContext, webview: vscode.Webview): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "history.js")
  );
  const styles = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "viewer.css")
  );
  const nonce = randomUUID().replaceAll("-", "");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<link rel="stylesheet" href="${styles}"><title>Compass Evolution</title></head>
<body><div id="root" role="status">Loading every reachable Git commit…</div>
<script nonce="${nonce}" src="${script}"></script></body></html>`;
}
