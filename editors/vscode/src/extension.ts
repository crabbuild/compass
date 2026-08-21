import path from "node:path";
import * as vscode from "vscode";
import type { CallDirection } from "@compass/viewer/contracts/callGraph";
import {
  COMPASS_REQUIREMENTS,
  compatibilityIssue,
  minimumCompassVersionIssue,
  type CapabilityRequirement
} from "./cli/compatibility";
import { CapabilityReportSchema } from "./cli/contracts";
import {
  discoverCompass,
  inspectCompassInstallation
} from "./cli/discovery";
import { CompassProcessManager } from "./cli/processManager";
import { CompassRuntime } from "./cli/runtime";
import { compassSelectionItems } from "./cli/selection";
import { registerBuildCommands } from "./commands/buildCommands";
import { resolveInstallCommand } from "./install/command";
import { GraphPanel } from "./views/graphPanel";
import {
  codeQueryRequiresRebuild,
  runCodeQuery,
  type CodeQueryRequest
} from "./views/codeQueryClient";
import { CallGraphPanel } from "./views/callGraphPanel";
import { runCallGraphAtCursor } from "./views/callGraphClient";
import { utf8ByteAt } from "./views/cursorByte";
import { openCallGraphGuidePanel } from "./views/callGraphGuidePanel";
import { openArchitecturePanel } from "./views/architecturePanel";
import { openQueryPanel } from "./views/queryPanel";
import { openHistoryPanel } from "./views/historyPanel";
import { openCliOnboardingPanel } from "./views/cliOnboardingPanel";
import { createCompassStatusBar } from "./views/statusBar";
import { WorkspaceTree } from "./views/workspaceTree";
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
    output.info(
      `Using Compass ${discovery.version ?? "(version unavailable)"} at ${executable}.`
    );
    if (discovery.installations.length > 1) {
      output.info(
        `Detected ${discovery.installations.length} Compass CLI installations.`
      );
    }
    await Promise.all(registry.all().map(async (session) => {
      try {
        session.capabilities = await processes.runJson(
          session.root,
          ["capabilities", "--format", "json"],
          CapabilityReportSchema
        );
        session.capabilityError = minimumCompassVersionIssue(
          session.capabilities.compass_version
        );
        if (session.capabilityError) output.warn(session.capabilityError);
      } catch (error) {
        session.capabilityError = message(error);
        output.warn(`Capability negotiation failed for ${executable}: ${session.capabilityError}`);
      }
    }));
  }

  const runtime = new CompassRuntime(discovery, {
    processes,
    sessions: () => registry.all(),
    persistCliPath: async (selectedPath) => {
      await vscode.workspace.getConfiguration("compass").update(
        "cliPath",
        selectedPath,
        vscode.ConfigurationTarget.Global
      );
    }
  });
  const workspaceTree = new WorkspaceTree(registry, () => runtime.discovery);
  const statusBar = createCompassStatusBar(context, registry);
  context.subscriptions.push(runtime.onDidChange(() => {
    workspaceTree.refresh();
    statusBar.refresh();
  }));
  const refresh = async () => {
    await registry.refresh();
    workspaceTree.refresh();
    statusBar.refresh();
  };
  const selectCompassBinary = async () => {
    const latestDiscovery = await discoverCompass(
      vscode.workspace.getConfiguration("compass")
    );
    const selected = await vscode.window.showQuickPick(
      compassSelectionItems(latestDiscovery),
      {
        title: "Select Compass CLI",
        placeHolder: latestDiscovery.installations.length > 0
          ? "Choose a detected Compass version or browse for another executable"
          : "No Compass installation was detected; browse for an executable",
        matchOnDescription: true,
        matchOnDetail: true
      }
    );
    if (!selected) return;

    let installation = selected.installation;
    let selectedPath: string | undefined;
    if (selected.browse) {
      const browsed = await vscode.window.showOpenDialog({
        title: "Select Compass CLI",
        canSelectFiles: true,
        canSelectFolders: false,
        canSelectMany: false
      });
      if (!browsed?.[0]) return;
      selectedPath = browsed[0].fsPath;
    } else if (selected.manual) {
      const configuredPath = vscode.workspace.getConfiguration("compass")
        .get<string>("cliPath")?.trim();
      selectedPath = await vscode.window.showInputBox({
        title: "Enter Compass CLI path",
        prompt: "Enter the full path to a Compass executable",
        placeHolder: "~/.local/bin/compass",
        ...(configuredPath ? { value: configuredPath } : {}),
        validateInput: (value) => value.trim()
          ? undefined
          : "Enter a Compass CLI path"
      });
      if (selectedPath === undefined) return;
    }
    if (selectedPath !== undefined) {
      installation = await inspectCompassInstallation(selectedPath);
      if (!installation) {
        void vscode.window.showErrorMessage(
          `The Compass CLI path is not an executable file: ${selectedPath}`
        );
        return;
      }
      if (!installation.version) {
        const action = await vscode.window.showWarningMessage(
          "Compass could not verify this executable with --version. Select it anyway?",
          "Use Anyway"
        );
        if (action !== "Use Anyway") return;
      }
    }
    if (!installation) return;
    if (installation.version) {
      const versionIssue = minimumCompassVersionIssue(installation.version);
      if (versionIssue) {
        void vscode.window.showErrorMessage(versionIssue);
        return;
      }
    }

    if (
      runtime.discovery.kind === "found"
      && installation.executable === runtime.discovery.executable
    ) {
      void vscode.window.showInformationMessage(
        `Compass ${installation.version ?? "(version unavailable)"} is already active.`
      );
      return;
    }
    try {
      const activated = await runtime.activate(installation);
      await refresh();
      void vscode.window.showInformationMessage(
        `Compass ${
          activated.installation.version ?? activated.capabilities.compass_version
        } is ready at ${activated.installation.executable}.`
      );
    } catch (error) {
      void vscode.window.showErrorMessage(
        `Compass CLI could not be activated: ${message(error)}`
      );
    }
  };
  const handleSetupAction = async (action: string | undefined) => {
    if (action === "Select Compass CLI") {
      await vscode.commands.executeCommand("compass.selectCli");
    } else if (action === "Open Setup") {
      await vscode.commands.executeCommand(
        "workbench.action.openWalkthrough",
        "crabbuild.crabbuild-compass-vscode#compass.getStarted",
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
      "Select Compass CLI",
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
  const openCompassSettings = async () => {
    const selected = await vscode.window.showQuickPick([
      {
        label: "$(settings-gear) Extension and watch settings",
        description: "CLI path, graph limits, debounce, and polling",
        action: "extension"
      },
      {
        label: "$(folder-library) Repository scope",
        description: "Review included and excluded paths",
        action: "scope"
      },
      {
        label: "$(file-code) Repository configuration",
        description: "Open .compass/config.toml",
        action: "configuration"
      },
      {
        label: "$(history) History profile",
        description: "Configure versioned graph profiles",
        action: "history"
      },
      {
        label: "$(terminal) Select Compass CLI",
        description: "Compare detected paths and installed versions",
        action: "cli"
      }
    ], {
      placeHolder: "Choose which Compass settings to configure"
    });
    if (!selected) return;
    if (selected.action === "extension") {
      await vscode.commands.executeCommand(
        "workbench.action.openSettings",
        "@ext:crabbuild.crabbuild-compass-vscode"
      );
      return;
    }
    if (selected.action === "cli") {
      await vscode.commands.executeCommand("compass.selectCli");
      return;
    }
    const session = await selectRepository();
    if (!session) return;
    if (selected.action === "scope") {
      await vscode.commands.executeCommand("compass.initialize", session.id);
      return;
    }
    if (selected.action === "history") {
      await vscode.commands.executeCommand("compass.openHistory", session.id);
      return;
    }
    const configurationUri = vscode.Uri.joinPath(
      vscode.Uri.file(session.root),
      ".compass",
      "config.toml"
    );
    try {
      await vscode.workspace.fs.stat(configurationUri);
      const document = await vscode.workspace.openTextDocument(configurationUri);
      await vscode.window.showTextDocument(document);
    } catch {
      const action = await vscode.window.showInformationMessage(
        "This repository does not have a Compass configuration yet.",
        "Configure scope"
      );
      if (action === "Configure scope") {
        await vscode.commands.executeCommand("compass.initialize", session.id);
      }
    }
  };
  const openCallGraph = async (direction: CallDirection) => {
    if (!vscode.workspace.isTrusted) {
      void vscode.window.showWarningMessage("Trust this workspace to run Compass.");
      return;
    }
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
      await CallGraphPanel.open(context, session, editor, output, direction);
    } catch (error) {
      void vscode.window.showErrorMessage(`Compass call graph failed: ${message(error)}`);
    }
  };
  const openCallGraphForSymbol = async (
    symbol: string,
    direction: CallDirection
  ): Promise<boolean> => {
    if (!vscode.workspace.isTrusted) {
      void vscode.window.showWarningMessage("Trust this workspace to run Compass.");
      return false;
    }
    const session = await selectRepository();
    if (!session) return false;
    if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.calls)) return false;
    if (session.graphState !== "available") {
      const action = await vscode.window.showInformationMessage(
        "Build the Compass code graph before tracing a symbol.",
        "Rebuild with Compass"
      );
      if (action === "Rebuild with Compass") {
        await vscode.commands.executeCommand("compass.update", session.id);
      }
      return false;
    }
    try {
      await CallGraphPanel.openForSymbol(
        context,
        session,
        symbol,
        output,
        direction
      );
      return true;
    } catch (error) {
      void vscode.window.showErrorMessage(
        `Compass call graph failed: ${message(error)}`
      );
      return false;
    }
  };
  const selectedSymbol = async (
    value: unknown,
    title: string
  ): Promise<string | undefined> => {
    if (typeof value === "string" && value.trim()) return value.trim();
    if (value && typeof value === "object") {
      const record = value as Record<string, unknown>;
      const candidate = typeof record.id === "string"
        ? record.id
        : typeof record.label === "string" ? record.label : undefined;
      if (candidate?.trim()) return candidate.trim();
    }
    const editor = vscode.window.activeTextEditor;
    const session = registry.forEditor(editor);
    if (editor && session) {
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.calls)) return undefined;
      if (session.graphState !== "available") {
        const action = await vscode.window.showInformationMessage(
          "Build the Compass code graph before resolving the symbol under the cursor.",
          "Rebuild with Compass"
        );
        if (action === "Rebuild with Compass") {
          await vscode.commands.executeCommand("compass.update", session.id);
        }
        return undefined;
      }
      const relativePath = path.relative(session.root, editor.document.uri.fsPath);
      if (
        path.isAbsolute(relativePath)
        || relativePath === ".."
        || relativePath.startsWith(`..${path.sep}`)
      ) {
        void vscode.window.showErrorMessage(
          "The active editor is outside the selected Compass repository."
        );
        return undefined;
      }
      const relative = relativePath.split(path.sep).join("/");
      try {
        const response = await runCallGraphAtCursor(
          session,
          {
            file: relative,
            byte: utf8ByteAt(editor.document, editor.selection.active),
            line: editor.selection.active.line + 1
          },
          "both",
          1
        );
        return response.rootSymbol;
      } catch (error) {
        void vscode.window.showErrorMessage(
          `Compass could not resolve a symbol at the cursor: ${message(error)}`
        );
        return undefined;
      }
    }
    const range = editor?.document.getWordRangeAtPosition(editor.selection.active);
    const word = editor && range ? editor.document.getText(range) : "";
    return vscode.window.showInputBox({
      title,
      prompt: "Enter a Compass symbol ID, name, or qualified name",
      ...(word ? { value: word } : {}),
      validateInput: (input) => input.trim() ? undefined : "Enter a symbol"
    });
  };
  const runAndOpenCodeQuery = async (
    request: CodeQueryRequest,
    repositoryId?: string
  ): Promise<void> => {
    if (!vscode.workspace.isTrusted) {
      void vscode.window.showWarningMessage("Trust this workspace to run Compass.");
      return;
    }
    const session = await selectRepository(repositoryId);
    if (!session) return;
    if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.query)) return;
    if (session.graphState !== "available") {
      const action = await vscode.window.showInformationMessage(
        "Build the Compass code graph before running code queries.",
        "Rebuild with Compass"
      );
      if (action === "Rebuild with Compass") {
        await vscode.commands.executeCommand("compass.update", session.id);
      }
      return;
    }
    const controller = new AbortController();
    try {
      const result = await runCodeQuery(session, request, controller.signal);
      await GraphPanel.open(
        context,
        session,
        output,
        result,
        codeQueryTitle(request, result.nodes)
      );
    } catch (error) {
      const detail = message(error);
      if (codeQueryRequiresRebuild(detail)) {
        const action = await vscode.window.showWarningMessage(
          detail,
          "Rebuild with Compass"
        );
        if (action === "Rebuild with Compass") {
          await vscode.commands.executeCommand("compass.update", session.id);
        }
      } else {
        void vscode.window.showErrorMessage(`Compass code query failed: ${detail}`);
      }
    }
  };
  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("compass.status", workspaceTree),
    vscode.window.onDidChangeActiveTextEditor(() => statusBar.refresh()),
    vscode.commands.registerCommand("compass.refreshWorkspace", refresh),
    vscode.commands.registerCommand("compass.selectCli", selectCompassBinary),
    vscode.commands.registerCommand("compass.installCli", () =>
      openCliOnboardingPanel(context, {
        runtime,
        selectExisting: selectCompassBinary,
        initializeRepository: async () => {
          await vscode.commands.executeCommand("compass.initialize");
        },
        refresh,
        discover: () => discoverCompass(vscode.workspace.getConfiguration("compass")),
        resolveCommand: () => resolveInstallCommand(process.platform, process.env)
      })
    ),
    vscode.commands.registerCommand("compass.openSettings", openCompassSettings),
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
    vscode.commands.registerCommand("compass.openCallGraphGuide", () => {
      openCallGraphGuidePanel(
        context,
        vscode.window.activeTextEditor,
        openCallGraphForSymbol
      );
    }),
    vscode.commands.registerCommand("compass.openCallGraph", () => openCallGraph("both")),
    vscode.commands.registerCommand("compass.openCallers", () => openCallGraph("callers")),
    vscode.commands.registerCommand("compass.openCallees", () => openCallGraph("callees")),
    vscode.commands.registerCommand(
      "compass.openCallersAndCallees",
      () => openCallGraph("both")
    ),
    vscode.commands.registerCommand("compass.searchSymbols", async () => {
      const query = await vscode.window.showInputBox({
        title: "Search Compass symbols",
        prompt: "Search typed symbol names",
        validateInput: (input) => input.trim() ? undefined : "Enter a search term"
      });
      if (query) await runAndOpenCodeQuery({ operation: "search", query });
    }),
    vscode.commands.registerCommand("compass.showCodeCallers", async (value?: unknown) => {
      if (value !== undefined) {
        const symbol = await selectedSymbol(value, "Show Compass callers");
        if (symbol) await runAndOpenCodeQuery({ operation: "callers", symbol });
        return;
      }
      await openCallGraph("callers");
    }),
    vscode.commands.registerCommand("compass.showCodeCallees", async (value?: unknown) => {
      if (value !== undefined) {
        const symbol = await selectedSymbol(value, "Show Compass callees");
        if (symbol) await runAndOpenCodeQuery({ operation: "callees", symbol });
        return;
      }
      await openCallGraph("callees");
    }),
    vscode.commands.registerCommand("compass.showCodeImpact", async (value?: unknown) => {
      const symbol = await selectedSymbol(value, "Show Compass impact");
      if (symbol) await runAndOpenCodeQuery({ operation: "impact", symbol });
    }),
    vscode.commands.registerCommand("compass.exploreCode", async (value?: unknown) => {
      const symbol = await selectedSymbol(value, "Explore related Compass symbols");
      if (symbol) await runAndOpenCodeQuery({ operation: "explore", symbols: [symbol] });
    }),
    vscode.commands.registerCommand("compass.showNodeTrail", async () => {
      const source = await selectedSymbol(undefined, "Compass node trail source");
      if (!source) return;
      const target = await vscode.window.showInputBox({
        title: "Compass node trail target",
        prompt: "Enter the destination symbol ID, name, or qualified name",
        validateInput: (value) => value.trim() ? undefined : "Enter a target symbol"
      });
      if (target) {
        await runAndOpenCodeQuery({ operation: "node", source, target: target.trim() });
      }
    }),
    vscode.commands.registerCommand("compass.openArchitecture", async (repositoryId?: string) => {
      const session = await selectRepository(repositoryId);
      if (!session) return;
      if (!await ensureCompatible(session, COMPASS_REQUIREMENTS.architecture)) return;
      await openArchitecturePanel(context, session, output);
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
      "Select Compass CLI",
      "Open Setup"
    ).then(handleSetupAction);
  } else if (discovery.kind === "missing") {
    void vscode.window.showInformationMessage(
      "Install Compass to build and explore a local code graph.",
      "Install Compass",
      "Select existing CLI"
    ).then(async (action) => {
      if (action === "Install Compass") {
        await vscode.commands.executeCommand("compass.installCli");
      } else if (action === "Select existing CLI") {
        await vscode.commands.executeCommand("compass.selectCli");
      }
    });
  }
}

function codeQueryTitle(
  request: CodeQueryRequest,
  nodes: readonly { id: string; name: string }[]
): string {
  const label = (symbol: string) =>
    nodes.find((node) => node.id === symbol)?.name ?? symbol;
  switch (request.operation) {
    case "search":
      return `Compass Search — ${request.query}`;
    case "callers":
      return `Compass Callers — ${label(request.symbol)}`;
    case "callees":
      return `Compass Callees — ${label(request.symbol)}`;
    case "impact":
      return `Compass Change Impact — ${label(request.symbol)}`;
    case "explore":
      return `Compass Related Symbols — ${request.symbols.map(label).join(", ")}`;
    case "node":
      return `Compass Node Trail — ${label(request.source)} to ${label(request.target)}`;
  }
}

export function deactivate(): void {}

function message(error: unknown): string {
  const detail = error instanceof Error ? error.message : String(error);
  return detail.split(/\r?\n/, 1)[0] ?? detail;
}
