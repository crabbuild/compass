import { createHash, randomUUID } from "node:crypto";
import * as vscode from "vscode";
import type { HistoryTimeline } from "@compass/viewer";
import { buildEnableHistoryArgs, buildHistoryArgs } from "../history/buildArguments";
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

const TIMELINE_PAGE_SIZE = 100;

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
  let timeline: HistoryTimeline | undefined;
  let initialization: Promise<void> | undefined;
  let timelinePageLoading = false;
  let timelineGeneration = 0;
  const graphNodeLimit = vscode.workspace
    .getConfiguration("compass")
    .get("graphNodeLimit", 5000);
  const countCache = new Map<string, Awaited<ReturnType<typeof loadChangeCounts>>>();
  let disposed = false;
  let activePanelBuild: {
    command: ReturnType<RepositorySession["processes"]["startJsonl"]>;
    cancelled: boolean;
  } | undefined;
  const postMessage = (message: HistoryHostMessage): Thenable<boolean> =>
    disposed ? Promise.resolve(false) : panel.webview.postMessage(message);
  const ensureInitialized = (): Promise<void> => {
    initialization ??= revisions.initialize();
    return initialization;
  };
  const pagedTimeline = session.capabilities?.features.history_timeline_pagination === true;
  const sendTimeline = async (): Promise<void> => {
    const generation = ++timelineGeneration;
    try {
      await ensureInitialized();
      const loaded = await loadTimeline(
        session,
        pagedTimeline ? { limit: TIMELINE_PAGE_SIZE } : undefined
      );
      if (generation !== timelineGeneration) return;
      timeline = loaded;
      await postMessage({ type: "timeline", timeline, repositoryId: session.id, generation });
    } catch (error) {
      if (generation !== timelineGeneration) return;
      initialization = undefined;
      const detail = error instanceof Error ? error.message : String(error);
      output.appendLine(`[history:error] ${detail}`);
      await postMessage({ type: "bootstrapError", message: detail });
    }
  };
  const sendMoreTimeline = async (): Promise<void> => {
    if (timelinePageLoading || !pagedTimeline || !timeline?.hasMore) return;
    const cursor = timeline.nextCursor ?? timeline.entries.at(-1)?.commit;
    if (!cursor) return;
    const generation = timelineGeneration;
    timelinePageLoading = true;
    try {
      const page = await loadTimeline(session, {
        limit: TIMELINE_PAGE_SIZE,
        after: cursor
      });
      if (generation !== timelineGeneration || !timeline) return;
      const loaded = new Set(timeline.entries.map((entry) => entry.commit));
      timeline = {
        ...page,
        entries: [
          ...timeline.entries,
          ...page.entries.filter((entry) => !loaded.has(entry.commit))
        ]
      };
      await postMessage({
        type: "timelinePage",
        timeline: page,
        repositoryId: session.id,
        generation
      });
    } catch (error) {
      if (generation !== timelineGeneration) return;
      const detail = error instanceof Error ? error.message : String(error);
      output.appendLine(`[history:error] ${detail}`);
      if (detail.includes("snapshot changed") || detail.includes("cursor is invalid")) {
        await sendTimeline();
      } else {
        await postMessage({ type: "timelinePageError", message: detail, generation });
      }
    } finally {
      timelinePageLoading = false;
    }
  };
  const refreshTimelineEntry = async (commit: string): Promise<void> => {
    const generation = ++timelineGeneration;
    let refreshedTimeline: HistoryTimeline;
    if (!pagedTimeline) {
      refreshedTimeline = await loadTimeline(session);
    } else {
      const refreshed = await loadTimeline(session, {
        limit: 1,
        revision: commit
      });
      const entry = refreshed.entries.find((candidate) => candidate.commit === commit);
      if (timeline && entry) {
        refreshedTimeline = {
          ...timeline,
          historyEnabled: refreshed.historyEnabled,
          entries: timeline.entries.map((candidate) =>
            candidate.commit === commit ? entry : candidate)
        };
      } else {
        refreshedTimeline = refreshed;
      }
    }
    if (generation !== timelineGeneration) return;
    timeline = refreshedTimeline;
    await postMessage({ type: "timeline", timeline, repositoryId: session.id, generation });
  };
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
      await postMessage({ type: "buildSucceeded", commit });
      try {
        await refreshTimelineEntry(commit);
      } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        output.appendLine(`[history:error] ${detail}`);
        await postMessage({
          type: "bootstrapError",
          message: `The revision graph was built, but commit history could not be refreshed: ${detail}`
        });
      }
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
  const enableHistory = async (): Promise<void> => {
    if (session.activeWriter) {
      await postMessage({
        type: "enableFailed",
        message: "Another Compass write operation is already running."
      });
      return;
    }
    const picked = await vscode.window.showQuickPick(
      [
        {
          label: "Code only",
          description: "Recommended · local AST and inferred evidence; no model credentials",
          value: "code-only" as const
        },
        {
          label: "Compass default profile",
          description: "Let the CLI resolve its configured provider; may be local or non-semantic",
          value: "default" as const
        }
      ],
      { title: "Enable Compass revision graphs" }
    );
    if (disposed) return;
    if (!picked) {
      await postMessage({ type: "enableCancelled" });
      return;
    }
    if (session.activeWriter) {
      await postMessage({
        type: "enableFailed",
        message: "Another Compass write operation started while choosing the history profile."
      });
      return;
    }
    const args = buildEnableHistoryArgs(picked.value);
    const command = session.processes.startCommand(session.root, args);
    let cancelled = false;
    session.activeWriter = command;
    await postMessage({ type: "enableRunning" });
    try {
      const result = await vscode.window.withProgress(
        {
          location: vscode.ProgressLocation.Notification,
          title: "Enabling Compass revision graphs",
          cancellable: true
        },
        async (_, token) => {
          token.onCancellationRequested(() => {
            cancelled = true;
            command.cancel();
          });
          output.appendLine(`> compass ${args.join(" ")}`);
          return command.completed;
        }
      );
      output.append(result.stdout);
      output.append(result.stderr);
      if (result.code !== 0) {
        if (cancelled || disposed) {
          await postMessage({ type: "enableCancelled" });
          return;
        }
        throw new Error(result.stderr || `Compass exited with ${result.code}`);
      }
      if (disposed) return;
      await postMessage({ type: "enableSucceeded" });
      await sendTimeline();
    } catch (error) {
      if (cancelled || disposed) {
        await postMessage({ type: "enableCancelled" });
      } else {
        const detail = error instanceof Error ? error.message : String(error);
        output.appendLine(`[history:error] ${detail}`);
        await postMessage({
          type: "enableFailed",
          message: conciseBuildError(detail)
        });
      }
    } finally {
      if (session.activeWriter?.operationId === command.operationId) {
        session.activeWriter = undefined;
      }
    }
  };
  panel.webview.onDidReceiveMessage(async (message) => {
    try {
      if (message?.type === "ready" || message?.type === "retryTimeline") {
        await sendTimeline();
      } else if (message?.type === "loadMoreTimeline") {
        await sendMoreTimeline();
      } else if (message?.type === "enableHistory") {
        await enableHistory();
      } else if (message?.type === "loadRevision" && typeof message.commit === "string") {
        const entry = timeline?.entries.find((candidate) => candidate.commit === message.commit);
        if (!entry) throw new Error("Reload commit history before opening this revision.");
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
        const currentEntry = timeline?.entries.find((entry) => entry.commit === message.commit);
        const parentEntry = timeline?.entries.find((entry) => entry.commit === message.parent);
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
        const entry = timeline?.entries.find((candidate) => candidate.commit === message.commit);
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
  panel.webview.html = html(context, panel.webview);
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
<body><div id="root"></div>
<script nonce="${nonce}" src="${script}"></script></body></html>`;
}
