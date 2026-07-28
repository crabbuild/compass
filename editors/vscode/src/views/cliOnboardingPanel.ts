import { randomUUID } from "node:crypto";
import * as vscode from "vscode";
import type { CliOnboardingState } from "@compass/viewer";
import type { CompassDiscovery, CompassInstallation } from "../cli/discovery";
import type { CompassRuntime } from "../cli/runtime";
import type { InstallCommand } from "../install/command";
import {
  OnboardingToHostMessageSchema,
  type HostToOnboardingMessage
} from "../install/messages";

export type CliOnboardingDependencies = {
  runtime: CompassRuntime;
  selectExisting(): Promise<void>;
  initializeRepository(): Promise<void>;
  refresh(): Promise<void>;
  discover(): Promise<CompassDiscovery>;
  resolveCommand(): Promise<InstallCommand>;
  pollIntervalMs?: number;
  pollTimeoutMs?: number;
};

let currentPanel: vscode.WebviewPanel | undefined;
let currentTerminal: vscode.Terminal | undefined;

export async function openCliOnboardingPanel(
  context: vscode.ExtensionContext,
  dependencies: CliOnboardingDependencies
): Promise<void> {
  if (currentPanel) {
    currentPanel.reveal(vscode.ViewColumn.Active);
    return;
  }

  const command = await dependencies.resolveCommand();
  const panel = vscode.window.createWebviewPanel(
    "compass.onboarding",
    "Get started with Compass",
    vscode.ViewColumn.Active,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")]
    }
  );
  currentPanel = panel;
  panel.webview.html = html(context, panel.webview);
  let disposed = false;
  let executionId = 0;
  let state = initialState(command, dependencies.runtime.discovery);
  const disposables: vscode.Disposable[] = [];
  const postState = async (next: CliOnboardingState): Promise<void> => {
    state = next;
    if (disposed) return;
    const message: HostToOnboardingMessage = { type: "state", state };
    await panel.webview.postMessage(message);
  };

  const verify = async (searchedFallback: readonly string[] = []): Promise<boolean> => {
    await postState({ kind: "verifying" });
    const discovery = await dependencies.discover();
    if (discovery.kind === "missing") {
      await postState({
        kind: "error",
        title: "Compass was not found",
        message: "The installer finished, but Compass was not found in a configured, PATH, or common install location.",
        searched: discovery.searched.length > 0
          ? discovery.searched.slice(0, 256).map((value) => value.slice(0, 8192))
          : [...searchedFallback].slice(0, 256).map((value) => value.slice(0, 8192)),
        canVerifyAgain: true
      });
      return false;
    }
    try {
      const installation = activeInstallation(discovery);
      const activated = await dependencies.runtime.activate(installation);
      await dependencies.refresh();
      await postState({
        kind: "ready",
        version: activated.installation.version
          ?? activated.capabilities.compass_version,
        executable: activated.installation.executable,
        hasWorkspace: (vscode.workspace.workspaceFolders?.length ?? 0) > 0
      });
      return true;
    } catch (error) {
      await postState({
        kind: "error",
        title: "Compass is not compatible",
        message: errorMessage(error),
        canVerifyAgain: true
      });
      return false;
    }
  };

  const pollForInstallation = async (run: number): Promise<void> => {
    const interval = dependencies.pollIntervalMs ?? 750;
    const timeout = dependencies.pollTimeoutMs ?? 120_000;
    const started = Date.now();
    while (!disposed && run === executionId && Date.now() - started < timeout) {
      const discovery = await dependencies.discover();
      if (discovery.kind === "found") {
        await verify(discovery.searched);
        return;
      }
      await delay(interval);
    }
    if (!disposed && run === executionId) {
      await postState({
        kind: "error",
        title: "Installation could not be verified",
        message: "Compass did not appear before verification timed out. Review the terminal output, then try again.",
        canVerifyAgain: true
      });
    }
  };

  const install = async (): Promise<void> => {
    if (
      command.kind !== "supported"
      || state.kind === "installing"
      || state.kind === "verifying"
    ) return;
    const run = ++executionId;
    await postState({
      kind: "installing",
      platform: command.platformLabel,
      command: command.command
    });
    const terminal = vscode.window.createTerminal({
      name: "Compass Setup",
      ...(command.shellPath ? { shellPath: command.shellPath } : {})
    });
    currentTerminal = terminal;
    terminal.show(false);
    const integration = await waitForShellIntegration(terminal, 1_500);
    if (disposed || run !== executionId) return;
    if (!integration) {
      terminal.sendText(command.command, true);
      await pollForInstallation(run);
      return;
    }

    const execution = integration.executeCommand(command.command);
    const exitCode = await waitForExecution(terminal, execution);
    if (disposed || run !== executionId) return;
    if (exitCode !== 0) {
      await postState({
        kind: "error",
        title: "Installation failed",
        message: exitCode === undefined
          ? "The terminal stopped before the installer reported an exit code."
          : `The installer exited with code ${exitCode}. Review the terminal output, then try again.`,
        canVerifyAgain: false
      });
      return;
    }
    await verify();
  };

  disposables.push(
    panel.webview.onDidReceiveMessage(async (raw) => {
      const parsed = OnboardingToHostMessageSchema.safeParse(raw);
      if (!parsed.success) return;
      switch (parsed.data.type) {
        case "ready":
          await postState(state);
          break;
        case "install":
          await install();
          break;
        case "verifyAgain":
          await verify();
          break;
        case "selectExisting":
          await dependencies.selectExisting();
          if (dependencies.runtime.discovery.kind === "found") {
            const active = dependencies.runtime.discovery;
            await postState({
              kind: "ready",
              version: active.version ?? "version unavailable",
              executable: active.executable,
              hasWorkspace: (vscode.workspace.workspaceFolders?.length ?? 0) > 0
            });
          }
          break;
        case "initializeRepository":
          await dependencies.initializeRepository();
          break;
        case "openRepository":
          await vscode.commands.executeCommand("vscode.openFolder");
          break;
        case "showTerminal":
          currentTerminal?.show(false);
          break;
      }
    }),
    vscode.window.onDidCloseTerminal((terminal) => {
      if (terminal !== currentTerminal) return;
      currentTerminal = undefined;
      if (state.kind === "installing") {
        executionId += 1;
        void postState({
          kind: "error",
          title: "Installation stopped",
          message: "The Compass Setup terminal closed before installation could be verified.",
          canVerifyAgain: true
        });
      }
    }),
    panel.onDidDispose(() => {
      disposed = true;
      executionId += 1;
      currentPanel = undefined;
      currentTerminal = undefined;
      for (const disposable of disposables) disposable.dispose();
    })
  );
}

function initialState(
  command: InstallCommand,
  discovery: CompassDiscovery
): CliOnboardingState {
  if (discovery.kind === "found") {
    return {
      kind: "ready",
      version: discovery.version ?? "version unavailable",
      executable: discovery.executable,
      hasWorkspace: (vscode.workspace.workspaceFolders?.length ?? 0) > 0
    };
  }
  return command.kind === "supported"
    ? {
      kind: "ready-to-install",
      platform: command.platformLabel,
      command: command.command
    }
    : {
      kind: "unsupported",
      platform: command.platformLabel,
      message: command.message
    };
}

function activeInstallation(discovery: Extract<CompassDiscovery, { kind: "found" }>)
  : CompassInstallation {
  return discovery.installations.find(
    (installation) => installation.executable === discovery.executable
  ) ?? {
    executable: discovery.executable,
    ...(discovery.version ? { version: discovery.version } : {}),
    source: "configured"
  };
}

function waitForShellIntegration(
  terminal: vscode.Terminal,
  timeoutMs: number
): Promise<vscode.TerminalShellIntegration | undefined> {
  if (terminal.shellIntegration) return Promise.resolve(terminal.shellIntegration);
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      disposable.dispose();
      resolve(undefined);
    }, timeoutMs);
    const disposable = vscode.window.onDidChangeTerminalShellIntegration((event) => {
      if (event.terminal !== terminal) return;
      clearTimeout(timer);
      disposable.dispose();
      resolve(event.shellIntegration);
    });
  });
}

function waitForExecution(
  terminal: vscode.Terminal,
  execution: vscode.TerminalShellExecution
): Promise<number | undefined> {
  return new Promise((resolve) => {
    const ended = vscode.window.onDidEndTerminalShellExecution((event) => {
      if (event.terminal !== terminal || event.execution !== execution) return;
      close.dispose();
      ended.dispose();
      resolve(event.exitCode);
    });
    const close = vscode.window.onDidCloseTerminal((closed) => {
      if (closed !== terminal) return;
      close.dispose();
      ended.dispose();
      resolve(undefined);
    });
  });
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function html(context: vscode.ExtensionContext, webview: vscode.Webview): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "onboarding.js")
  );
  const styles = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "viewer.css")
  );
  const onboardingStyles = webview.asWebviewUri(
    vscode.Uri.joinPath(context.extensionUri, "dist", "webviews", "onboarding.css")
  );
  const nonce = randomUUID().replaceAll("-", "");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<link rel="stylesheet" href="${styles}">
<link rel="stylesheet" href="${onboardingStyles}">
<title>Get started with Compass</title></head>
<body><div id="root"></div><script nonce="${nonce}" src="${script}"></script></body></html>`;
}
