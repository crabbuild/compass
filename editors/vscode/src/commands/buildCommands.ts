import * as vscode from "vscode";
import { COMPASS_REQUIREMENTS, type CapabilityRequirement } from "../cli/compatibility";
import { buildInitArgs, buildUpdateArgs, buildWatchArgs } from "./buildArguments";
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
  const pick = (repositoryId?: string) => pickRepository(registry, repositoryId);
  context.subscriptions.push(
    vscode.commands.registerCommand("compass.initialize", async (repositoryId?: string) => {
      const session = await pick(repositoryId);
      if (!session) return;
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.initialize)) return;
      const includes = await vscode.window.showInputBox({
        title: "Compass: Initialize Repository",
        prompt: "Include globs, comma separated (leave empty for everything)"
      });
      if (includes === undefined) return;
      const excludes = await vscode.window.showInputBox({
        title: "Compass: Initialize Repository",
        prompt: "Exclude globs, comma separated"
      });
      if (excludes === undefined) return;
      const confirmed = await vscode.window.showWarningMessage(
        `Initialize Compass in ${session.root}?`,
        { modal: true, detail: `Includes: ${includes || "all"}\nExcludes: ${excludes || "none"}` },
        "Initialize"
      );
      if (confirmed !== "Initialize") return;
      await runGuided(session, buildInitArgs({
        root: session.root,
        includes: splitGlobs(includes),
        excludes: splitGlobs(excludes),
        force: false
      }), "Initializing Compass", output, refresh);
    }),
    vscode.commands.registerCommand("compass.update", async (repositoryId?: string) => {
      const session = await pick(repositoryId);
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
    vscode.commands.registerCommand("compass.toggleWatch", async (repositoryId?: string) => {
      const session = await pick(repositoryId);
      if (!session) return;
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.watch)) return;
      if (session.watch) {
        session.watch.cancel();
        session.watch = undefined;
        void vscode.window.showInformationMessage("Compass watch stopped.");
        await refresh();
        return;
      }
      output.appendLine(`> compass ${buildWatchArgs({
        root: session.root,
        debounceSeconds: 0.4,
        poll: false
      }).join(" ")}`);
      const command = session.processes.startJsonl(
        session.root,
        buildWatchArgs({ root: session.root, debounceSeconds: 0.4, poll: false }),
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
    })
  );
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
  repositoryId?: string
): Promise<RepositorySession | undefined> {
  const requested = registry.byId(repositoryId);
  if (requested) return requested;
  const sessions = registry.all();
  if (sessions.length === 0) {
    void vscode.window.showInformationMessage("Open a repository folder to use Compass.");
    return undefined;
  }
  if (sessions.length === 1) return sessions[0];
  const picked = await vscode.window.showQuickPick(
    sessions.map((session) => ({ label: session.root, session })),
    { placeHolder: "Choose the repository Compass should modify" }
  );
  return picked?.session;
}

function splitGlobs(value: string): string[] {
  return value.split(",").map((item) => item.trim()).filter(Boolean);
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
