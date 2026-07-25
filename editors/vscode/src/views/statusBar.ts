import * as vscode from "vscode";
import type { SessionRegistry } from "../workspace/sessionRegistry";

export function createCompassStatusBar(
  context: vscode.ExtensionContext,
  registry: SessionRegistry
): { refresh(): void } {
  const item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 20);
  item.command = "compass.openGraph";
  item.name = "Compass";
  context.subscriptions.push(item);
  return {
    refresh() {
      const session = registry.forEditor(vscode.window.activeTextEditor);
      if (!session) {
        item.hide();
        return;
      }
      item.text = session.graphState === "building" ? "$(sync~spin) Compass"
        : session.graphState === "available" ? "$(compass) Compass"
          : session.graphState === "failed" ? "$(error) Compass"
            : "$(circle-large-outline) Compass";
      item.tooltip = `Compass: ${session.graphState.replace("-", " ")}`;
      item.show();
    }
  };
}
