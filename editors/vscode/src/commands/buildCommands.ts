import * as vscode from "vscode";
import { COMPASS_REQUIREMENTS, type CapabilityRequirement } from "../cli/compatibility";
import { buildUpdateArgs, buildWatchArgs } from "./buildArguments";
import { openInitializationPanel } from "../views/initializationPanel";
import type { RepositorySession } from "../workspace/repositorySession";
import type { SessionRegistry } from "../workspace/sessionRegistry";

export function registerBuildCommands(
  context: vscode.ExtensionContext,
  registry: SessionRegistry,
  output: vscode.OutputChannel,
  refresh: () => Promise<void>,
  ensureCompatible: (
    session: RepositorySession,
    requirement: CapabilityRequirement
  ) => Promise<boolean>
): void {
  const pick = (
    repositoryId: string | undefined,
    eligible: (session: RepositorySession) => boolean
  ) => pickRepository(registry, repositoryId, eligible);
  context.subscriptions.push(
    vscode.commands.registerCommand("compass.initialize", async (repositoryId?: string) => {
      const session = await pick(
        repositoryId,
        (candidate) => candidate.graphState === "not-materialized"
      );
      if (!session) return;
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.initialize)) return;
      await openInitializationPanel(context, session, output, refresh);
    }),
    vscode.commands.registerCommand("compass.update", async (target?: unknown) => {
      const repositoryId = repositoryIdFromTarget(target);
      const session = await pick(
        repositoryId,
        (candidate) => candidate.graphState === "available" || candidate.graphState === "failed"
      );
      if (!session) return;
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.update)) return;
      if (session.watch) {
        const stop = await vscode.window.showWarningMessage(
          "Compass watch is active. Stop it before updating?",
          "Stop and Update"
        );
        if (stop !== "Stop and Update") return;
        session.watch.cancel();
        session.watch = undefined;
      }
      await runGuided(
        session,
        buildUpdateArgs({ root: session.root, noViz: true }),
        "Updating Compass graph",
        output,
        refresh
      );
    }),
    vscode.commands.registerCommand(
      "compass.toggleWatch",
      (target?: unknown) => setWatch(target)
    ),
    vscode.commands.registerCommand(
      "compass.startWatch",
      (target?: unknown) => setWatch(target, true)
    ),
    vscode.commands.registerCommand(
      "compass.stopWatch",
      (target?: unknown) => setWatch(target, false)
    )
  );

  async function setWatch(target: unknown, requestedState?: boolean): Promise<void> {
    const repositoryId = repositoryIdFromTarget(target);
    const session = await pick(
      repositoryId,
      (candidate) => candidate.graphState === "available" || candidate.watch !== undefined
    );
    if (!session) return;
    if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.watch)) return;
    if (requestedState === Boolean(session.watch)) return;
    if (session.watch) {
      session.watch.cancel();
      session.watch = undefined;
      void vscode.window.showInformationMessage("Compass watch stopped.");
      await refresh();
      return;
    }
    const configuration = vscode.workspace.getConfiguration(
      "compass",
      vscode.Uri.file(session.root)
    );
    const debounceSeconds = configuration.get<number>("watch.debounceSeconds", 0.4);
    const poll = configuration.get<boolean>("watch.poll", false);
    output.appendLine(`> compass ${buildWatchArgs({
      root: session.root,
      debounceSeconds,
      poll
    }).join(" ")}`);
    const command = session.processes.startJsonl(
      session.root,
      buildWatchArgs({ root: session.root, debounceSeconds, poll }),
      (event) => output.appendLine(`[${event.phase}] ${event.message}`)
    );
    session.watch = command;
    void command.completed.finally(async () => {
      if (session.watch?.operationId === command.operationId) {
        session.watch = undefined;
      }
      await refresh();
    });
    void vscode.window.showInformationMessage("Compass watch started.");
    await refresh();
  }
}

function repositoryIdFromTarget(target: unknown): string | undefined {
  if (typeof target === "string") return target;
  if (
    typeof target === "object"
    && target !== null
    && "repositoryId" in target
    && typeof target.repositoryId === "string"
  ) {
    return target.repositoryId;
  }
  return undefined;
}

async function runGuided(
  session: RepositorySession,
  args: string[],
  title: string,
  output: vscode.OutputChannel,
  refresh: () => Promise<void>
): Promise<void> {
  if (session.activeWriter) {
    void vscode.window.showWarningMessage("A Compass write operation is already running.");
    return;
  }
  const command = session.processes.startJsonl(
    session.root,
    args,
    (event) => output.appendLine(`[${event.phase}] ${event.message}`)
  );
  session.activeWriter = command;
  session.graphState = "building";
  await refresh();
  try {
    const result = await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title, cancellable: true },
      async (_, token) => {
        token.onCancellationRequested(() => command.cancel());
        output.appendLine(`> compass ${args.join(" ")}`);
        return command.completed;
      }
    );
    output.append(result.stdout);
    output.append(result.stderr);
    if (result.code !== 0) throw new Error(result.stderr || `Compass exited with ${result.code}`);
    session.graphState = "available";
  } catch (error) {
    session.graphState = "failed";
    void vscode.window.showErrorMessage(`Compass failed: ${message(error)}`);
  } finally {
    if (session.activeWriter?.operationId === command.operationId) {
      session.activeWriter = undefined;
    }
    await refresh();
  }
}

async function pickRepository(
  registry: SessionRegistry,
  repositoryId: string | undefined,
  eligible: (session: RepositorySession) => boolean
): Promise<RepositorySession | undefined> {
  const requested = registry.byId(repositoryId);
  if (requested) return requested;
  const editor = vscode.window.activeTextEditor;
  const fromEditor = editor ? registry.forEditor(editor) : undefined;
  if (fromEditor && eligible(fromEditor)) return fromEditor;
  const sessions = registry.all().filter(eligible);
  if (sessions.length === 0) {
    void vscode.window.showInformationMessage(
      "No open repository is ready for this Compass action."
    );
    return undefined;
  }
  if (sessions.length === 1) return sessions[0];
  const picked = await vscode.window.showQuickPick(
    sessions.map((session) => ({ label: session.root, session })),
    { placeHolder: "Choose the repository Compass should modify" }
  );
  return picked?.session;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
