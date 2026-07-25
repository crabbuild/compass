import * as vscode from "vscode";
import {
  COMPASS_REQUIREMENTS,
  compatibilityIssue,
  type CapabilityRequirement
} from "./cli/compatibility";
import { CapabilityReportSchema } from "./cli/contracts";
import { discoverCompass } from "./cli/discovery";
import { CompassProcessManager } from "./cli/processManager";
import { registerBuildCommands } from "./commands/buildCommands";
import { GraphPanel } from "./views/graphPanel";
import { CallGraphPanel } from "./views/callGraphPanel";
import { openArchitecturePanel } from "./views/architecturePanel";
import { openQueryPanel } from "./views/queryPanel";
import { openHistoryPanel } from "./views/historyPanel";
import { OperationsTree } from "./views/operationsTree";
import { StatusTree } from "./views/statusTree";
import { createCompassStatusBar } from "./views/statusBar";
import { SessionRegistry } from "./workspace/sessionRegistry";

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel("Compass", { log: true });
  context.subscriptions.push(output);
  const discovery = await discoverCompass(vscode.workspace.getConfiguration("compass"));
  const executable = discovery.kind === "found" ? discovery.executable : "compass";
  const processes = new CompassProcessManager(executable);
  const registry = new SessionRegistry(vscode.workspace.workspaceFolders ?? [], processes);
  await registry.refresh();

  if (discovery.kind === "found") {
    await Promise.all(registry.all().map(async (session) => {
      try {
        session.capabilities = await processes.runJson(
          session.root,
          ["capabilities", "--format", "json"],
          CapabilityReportSchema
        );
      } catch (error) {
        session.capabilityError = message(error);
        output.warn(`Capability negotiation failed for ${executable}: ${session.capabilityError}`);
      }
    }));
  }

  const statusTree = new StatusTree(registry, discovery);
  const operationsTree = new OperationsTree(registry);
  const statusBar = createCompassStatusBar(context, registry);
  const refresh = async () => {
    await registry.refresh();
    statusTree.refresh();
    operationsTree.refresh();
    statusBar.refresh();
  };
  const selectCompassBinary = async () => {
    const selected = await vscode.window.showOpenDialog({
      title: "Select Compass CLI",
      canSelectFiles: true,
      canSelectFolders: false,
      canSelectMany: false
    });
    if (!selected?.[0]) return;
    await vscode.workspace.getConfiguration("compass").update(
      "cliPath",
      selected[0].fsPath,
      vscode.ConfigurationTarget.Global
    );
    const action = await vscode.window.showInformationMessage(
      "Compass CLI selected. Reload VS Code to activate it.",
      "Reload Window"
    );
    if (action === "Reload Window") {
      await vscode.commands.executeCommand("workbench.action.reloadWindow");
    }
  };
  const handleSetupAction = async (action: string | undefined) => {
    if (action === "Select Compass Binary") {
      await vscode.commands.executeCommand("compass.selectCli");
    } else if (action === "Open Setup") {
      await vscode.commands.executeCommand(
        "workbench.action.openWalkthrough",
        "crabbuild.compass-vscode#compass.getStarted",
        false
      );
    }
  };
  const ensureCompatible = async (
    session: ReturnType<SessionRegistry["all"]>[number],
    requirement: CapabilityRequirement
  ): Promise<boolean> => {
    const issue = compatibilityIssue(
      session.capabilities,
      session.capabilityError,
      requirement
    );
    if (!issue) return true;
    const action = await vscode.window.showErrorMessage(
      `${issue} Upgrade Compass or select a newer Compass binary, then reload VS Code.`,
      "Select Compass Binary",
      "Open Setup"
    );
    await handleSetupAction(action);
    return false;
  };
  const selectRepository = async (repositoryId?: string) => {
    const requested = registry.byId(repositoryId);
    if (requested) return requested;
    const editor = vscode.window.activeTextEditor;
    const fromEditor = editor ? registry.forEditor(editor) : undefined;
    if (fromEditor) return fromEditor;
    const sessions = registry.all();
    if (sessions.length === 0) {
      void vscode.window.showInformationMessage("Open a repository folder first.");
      return undefined;
    }
    if (sessions.length === 1) return sessions[0];
    const picked = await vscode.window.showQuickPick(
      sessions.map((session) => ({
        label: vscode.workspace.asRelativePath(session.root),
        description: session.root,
        session
      })),
      { placeHolder: "Choose the repository Compass should open" }
    );
    return picked?.session;
  };
  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("compass.status", statusTree),
    vscode.window.registerTreeDataProvider("compass.operations", operationsTree),
    vscode.window.onDidChangeActiveTextEditor(() => statusBar.refresh()),
    vscode.commands.registerCommand("compass.selectCli", selectCompassBinary),
    vscode.commands.registerCommand("compass.openGraph", async (repositoryId?: string) => {
      if (!vscode.workspace.isTrusted) {
        void vscode.window.showWarningMessage("Trust this workspace to run Compass.");
        return;
      }
      const session = await selectRepository(repositoryId);
      if (!session) {
        return;
      }
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.graph)) return;
      if (session.graphState !== "available") {
        const action = await vscode.window.showInformationMessage(
          "This repository does not have a materialized Compass graph.",
          "Initialize"
        );
        if (action === "Initialize") {
          await vscode.commands.executeCommand("compass.initialize", session.id);
        }
        return;
      }
      await GraphPanel.open(context, session, output);
    }),
    vscode.commands.registerCommand("compass.openCallGraph", async () => {
      const editor = vscode.window.activeTextEditor;
      const session = registry.forEditor(editor);
      if (!editor || !session) {
        void vscode.window.showInformationMessage(
          "Place the cursor inside a repository function to open its call graph."
        );
        return;
      }
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.calls)) return;
      try {
        await CallGraphPanel.open(context, session, editor);
      } catch (error) {
        void vscode.window.showErrorMessage(`Compass call graph failed: ${message(error)}`);
      }
    }),
    vscode.commands.registerCommand("compass.openArchitecture", async (repositoryId?: string) => {
      const session = await selectRepository(repositoryId);
      if (!session) return;
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.architecture)) return;
      await openArchitecturePanel(context, session);
    }),
    vscode.commands.registerCommand("compass.openQuery", async (repositoryId?: string) => {
      const session = await selectRepository(repositoryId);
      if (!session) return;
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.query)) return;
      await openQueryPanel(context, session);
    }),
    vscode.commands.registerCommand("compass.openHistory", async (repositoryId?: string) => {
      const session = await selectRepository(repositoryId);
      if (!session) return;
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.history)) return;
      await openHistoryPanel(context, session, output);
    })
  );
  registerBuildCommands(context, registry, output, refresh, ensureCompatible);
  statusBar.refresh();

  const incompatible = registry.all().find((session) => session.capabilityError);
  if (discovery.kind === "found" && incompatible) {
    void vscode.window.showWarningMessage(
      `The Compass CLI at ${executable} is not compatible with this extension. ${incompatible.capabilityError}`,
      "Select Compass Binary",
      "Open Setup"
    ).then(handleSetupAction);
  } else if (discovery.kind === "missing") {
    void vscode.window.showInformationMessage(
      "Compass CLI is required. Install it, then select or configure the executable.",
      "Open Setup",
      "Select Compass Binary"
    ).then(handleSetupAction);
  }
}

export function deactivate(): void {}

function message(error: unknown): string {
  const detail = error instanceof Error ? error.message : String(error);
  return detail.split(/\r?\n/, 1)[0] ?? detail;
}
