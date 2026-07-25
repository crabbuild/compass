import * as vscode from "vscode";
import type { SessionRegistry } from "../workspace/sessionRegistry";

export class StatusTree implements vscode.TreeDataProvider<vscode.TreeItem> {
  private readonly changes = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changes.event;

  constructor(
    private readonly registry: SessionRegistry,
    private readonly cliLabel: string
  ) {}

  refresh(): void {
    this.changes.fire();
  }

  getTreeItem(item: vscode.TreeItem): vscode.TreeItem {
    return item;
  }

  getChildren(): vscode.TreeItem[] {
    const cli = new vscode.TreeItem("Compass CLI", vscode.TreeItemCollapsibleState.None);
    cli.description = this.cliLabel;
    cli.iconPath = new vscode.ThemeIcon(this.cliLabel === "Not found" ? "warning" : "check");
    const repositories = this.registry.all().map((session) => {
      const item = new vscode.TreeItem(session.root, vscode.TreeItemCollapsibleState.None);
      item.description = stateLabel(session.graphState);
      item.iconPath = new vscode.ThemeIcon(
        session.graphState === "available" ? "pass"
          : session.graphState === "building" ? "sync~spin"
            : session.graphState === "failed" ? "error" : "circle-large-outline"
      );
      item.command = session.graphState === "available"
        ? { command: "compass.openGraph", title: "Open code graph" }
        : { command: "compass.initialize", title: "Initialize Compass" };
      return item;
    });
    return [cli, ...repositories];
  }
}

function stateLabel(state: string): string {
  return state === "available" ? "Graph available"
    : state === "not-materialized" ? "Not materialized"
      : state === "building" ? "Building"
        : "Failed";
}
