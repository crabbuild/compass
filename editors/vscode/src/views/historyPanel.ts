import { createHash, randomUUID } from "node:crypto";
import * as vscode from "vscode";
import { buildHistoryArgs } from "../history/buildArguments";
import { loadSemanticDiff } from "../history/diffClient";
import { loadChangeCounts } from "../history/changeCountsClient";
import { RevisionStore } from "../history/revisionStore";
import { loadTimeline } from "../history/timelineClient";
import {
  historyOperationFor,
  type HistoryHostMessage
} from "../history/panelMessages";
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
  const graphNodeLimit = vscode.workspace
    .getConfiguration("compass")
    .get("graphNodeLimit", 5000);
  const countCache = new Map<string, Awaited<ReturnType<typeof loadChangeCounts>>>();
  panel.webview.html = html(context, panel.webview);
  let disposed = false;
  let activePanelBuild: {
    command: ReturnType<RepositorySession["processes"]["startJsonl"]>;
    cancelled: boolean;
  } | undefined;
  const postMessage = (message: HistoryHostMessage): Thenable<boolean> =>
    disposed ? Promise.resolve(false) : panel.webview.postMessage(message);
  panel.onDidDispose(() => {
    disposed = true;
    if (activePanelBuild) {
      activePanelBuild.cancelled = true;
      activePanelBuild.command.cancel();
    }
  });
  const buildRevision = async (commit: string): Promise<void> => {
    if (disposed) return;
    if (session.activeWriter) {
      await postMessage({
        type: "buildFailed",
        commit,
        message: "Another Compass write operation is already running."
      });
      return;
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
      { title: `Build graph for ${commit.slice(0, 9)}` }
    );
    if (disposed) return;
    if (!profile) {
      await postMessage({ type: "buildCancelled", commit });
      return;
    }
    let selectedProfile:
      | { kind: "configured" | "code-only" }
      | { kind: "from"; source: string };
    if (profile.value.kind === "from") {
      const source = await vscode.window.showInputBox({
        title: "Reuse Compass history profile",
        prompt: "Enter a revision or realization ID"
      });
      if (disposed) return;
      if (source === undefined) {
        await postMessage({ type: "buildCancelled", commit });
        return;
      }
      selectedProfile = { kind: "from", source };
    } else {
      selectedProfile = profile.value;
    }

    let command: ReturnType<RepositorySession["processes"]["startJsonl"]> | undefined;
    let buildAttempt: typeof activePanelBuild;
    try {
      if (session.activeWriter) {
        throw new Error("Another Compass write operation started while choosing the history profile.");
      }
      const runningCommand = session.processes.startJsonl(
        session.root,
        buildHistoryArgs({
          revision: commit,
          all: false,
          firstParent: false,
          profile: selectedProfile
        }),
        (event) => output.appendLine(`[history:${event.phase}] ${event.message}`)
      );
      command = runningCommand;
      buildAttempt = { command: runningCommand, cancelled: false };
      activePanelBuild = buildAttempt;
      session.activeWriter = runningCommand;
      await postMessage({ type: "buildRunning", commit });
      const result = await vscode.window.withProgress(
        {
          location: vscode.ProgressLocation.Notification,
          title: `Building Compass graph for ${commit.slice(0, 9)}`,
          cancellable: true
        },
        async (_, token) => {
          token.onCancellationRequested(() => {
            if (buildAttempt) buildAttempt.cancelled = true;
            runningCommand.cancel();
          });
          return runningCommand.completed;
        }
      );
      if (buildAttempt.cancelled || disposed) {
        await postMessage({ type: "buildCancelled", commit });
        return;
      }
      output.append(result.stdout);
      output.append(result.stderr);
      if (result.code !== 0) {
        throw new Error(result.stderr || `Compass exited with ${result.code}`);
      }
      timeline = await loadTimeline(session);
      await postMessage({ type: "timeline", timeline, repositoryId: session.id });
      await postMessage({ type: "buildSucceeded", commit });
    } catch (error) {
      if (command && !buildAttempt?.cancelled) command.cancel();
      if (buildAttempt?.cancelled || disposed) {
        await postMessage({ type: "buildCancelled", commit });
      } else {
        const fullMessage = error instanceof Error ? error.message : String(error);
        output.appendLine(`[history:error] ${fullMessage}`);
        await postMessage({
          type: "buildFailed",
          commit,
          message: conciseBuildError(fullMessage)
        });
      }
    } finally {
      if (command && activePanelBuild?.command.operationId === command.operationId) {
        activePanelBuild = undefined;
      }
      if (command && session.activeWriter?.operationId === command.operationId) {
        session.activeWriter = undefined;
      }
    }
  };
  panel.webview.onDidReceiveMessage(async (message) => {
    try {
      if (message?.type === "ready") {
        await postMessage({ type: "timeline", timeline, repositoryId: session.id });
      } else if (message?.type === "loadRevision" && typeof message.commit === "string") {
        const entry = timeline.entries.find((candidate) => candidate.commit === message.commit);
        const revision = await revisions.load(
          message.commit,
          graphNodeLimit,
          historyIdentity(entry)
        );
        await postMessage({
          type: "graph",
          commit: message.commit,
          realization: revision.realization,
          fingerprint: revision.fingerprint,
          graph: revision.graph
        });
      } else if (message?.type === "openCommunity"
        && typeof message.commit === "string"
        && typeof message.communityId === "number"
        && typeof message.requestId === "string") {
        if (session.capabilities?.features.community_detail !== true) {
          throw new Error(
            "The installed Compass CLI does not support historical community details. Upgrade Compass and reload VS Code."
          );
        }
        if (typeof message.realization !== "string"
          || typeof message.fingerprint !== "string") {
          throw new Error("The historical graph identity is missing. Reopen the revision and try again.");
        }
        const expected = {
          realization: message.realization,
          fingerprint: message.fingerprint
        };
        const revision = await revisions.loadCommunity(
          message.commit,
          message.communityId,
          graphNodeLimit,
          expected
        );
        await postMessage({
          type: "communityGraph",
          requestId: message.requestId,
          commit: message.commit,
          communityId: message.communityId,
          graph: revision.graph
        });
      } else if (message?.type === "buildRevision" && typeof message.commit === "string") {
        await buildRevision(message.commit);
      } else if (message?.type === "compare"
        && typeof message.commit === "string"
        && typeof message.parent === "string") {
        const currentEntry = timeline.entries.find((entry) => entry.commit === message.commit);
        const parentEntry = timeline.entries.find((entry) => entry.commit === message.parent);
        if (!currentEntry?.presentationAvailable || !parentEntry?.presentationAvailable) {
          throw new Error("Both revisions must have graph available before comparison.");
        }
        const [current, parent, semanticDiff] = await Promise.all([
          revisions.load(message.commit, graphNodeLimit, historyIdentity(currentEntry)),
          revisions.load(message.parent, graphNodeLimit, historyIdentity(parentEntry)),
          loadSemanticDiff(session, message.parent, message.commit)
        ]);
        await postMessage({
          type: "comparison",
          commit: message.commit,
          parent: message.parent,
          realization: current.realization,
          fingerprint: current.fingerprint,
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
        await postMessage({ type: "changeCounts", commit: message.commit, counts });
      } else if (message?.type === "openSource") {
        await openGraphSource(session, message.repositoryId, message.source);
      }
    } catch (error) {
      if (message?.type === "buildRevision" && typeof message.commit === "string") {
        const fullMessage = error instanceof Error ? error.message : String(error);
        output.appendLine(`[history:error] ${fullMessage}`);
        await postMessage({
          type: "buildFailed",
          commit: message.commit,
          message: conciseBuildError(fullMessage)
        });
        return;
      }
      if (message?.type === "openCommunity"
        && typeof message.requestId === "string"
        && typeof message.communityId === "number") {
        await postMessage({
          type: "communityError",
          requestId: message.requestId,
          commit: typeof message.commit === "string" ? message.commit : "",
          communityId: message.communityId,
          message: error instanceof Error ? error.message : String(error)
        });
        return;
      }
      const commit = typeof message?.commit === "string" ? message.commit : undefined;
      await postMessage({
        type: "error",
        operation: historyOperationFor(message),
        ...(commit ? { commit } : {}),
        message: error instanceof Error ? error.message : String(error)
      });
    }
  });
}

function conciseBuildError(message: string): string {
  const normalized = message.replace(/\s+/g, " ").trim() || "Compass build failed.";
  return normalized.length <= 320 ? normalized : `${normalized.slice(0, 317)}…`;
}

function historyIdentity(
  entry: { realization: string | null; fingerprint: string | null } | undefined
): { realization: string; fingerprint: string } | undefined {
  return entry?.realization && entry.fingerprint
    ? { realization: entry.realization, fingerprint: entry.fingerprint }
    : undefined;
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
