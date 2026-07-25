import * as vscode from "vscode";
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
        output.warn(`Capability negotiation failed: ${message(error)}`);
      }
    }));
  }

  const statusTree = new StatusTree(
    registry,
    discovery.kind === "found" ? executable : "Not found"
  );
  const operationsTree = new OperationsTree(registry);
  const statusBar = createCompassStatusBar(context, registry);
  const refresh = async () => {
    await registry.refresh();
    statusTree.refresh();
    operationsTree.refresh();
    statusBar.refresh();
  };
  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("compass.status", statusTree),
    vscode.window.registerTreeDataProvider("compass.operations", operationsTree),
    vscode.window.onDidChangeActiveTextEditor(() => statusBar.refresh()),
    vscode.commands.registerCommand("compass.openGraph", async () => {
      if (!vscode.workspace.isTrusted) {
        void vscode.window.showWarningMessage("Trust this workspace to run Compass.");
        return;
      }
      const session = registry.forEditor(vscode.window.activeTextEditor);
      if (!session) {
        void vscode.window.showInformationMessage("Open a repository folder first.");
        return;
      }
      if (session.graphState !== "available") {
        const action = await vscode.window.showInformationMessage(
          "This repository does not have a materialized Compass graph.",
          "Initialize"
        );
        if (action === "Initialize") await vscode.commands.executeCommand("compass.initialize");
        return;
      }
      await GraphPanel.open(context, session);
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
      try {
        await CallGraphPanel.open(context, session, editor);
      } catch (error) {
        void vscode.window.showErrorMessage(`Compass call graph failed: ${message(error)}`);
      }
    }),
    vscode.commands.registerCommand("compass.openArchitecture", async () => {
      const session = registry.forEditor(vscode.window.activeTextEditor);
      if (!session) return;
      await openArchitecturePanel(context, session);
    }),
    vscode.commands.registerCommand("compass.openQuery", async () => {
      const session = registry.forEditor(vscode.window.activeTextEditor);
      if (!session) return;
      await openQueryPanel(context, session);
    }),
    vscode.commands.registerCommand("compass.openHistory", async () => {
      const session = registry.forEditor(vscode.window.activeTextEditor);
      if (!session) return;
      await openHistoryPanel(context, session, output);
    })
  );
  registerBuildCommands(context, registry, output, refresh);
  statusBar.refresh();

  if (discovery.kind === "missing") {
    const action = await vscode.window.showInformationMessage(
      "Compass CLI is required. Install it, then select or configure the executable.",
      "Open Setup",
      "Select Compass Binary"
    );
    if (action === "Select Compass Binary") {
      const selected = await vscode.window.showOpenDialog({
        title: "Select Compass CLI",
        canSelectFiles: true,
        canSelectFolders: false,
        canSelectMany: false
      });
      if (selected?.[0]) {
        await vscode.workspace.getConfiguration("compass").update(
          "cliPath",
          selected[0].fsPath,
          vscode.ConfigurationTarget.Global
        );
        void vscode.window.showInformationMessage("Compass CLI selected. Reload VS Code to activate it.");
      }
    }
  }
}

export function deactivate(): void {}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
