import * as vscode from "vscode";
import type { SessionRegistry } from "../workspace/sessionRegistry";

export class OperationsTree implements vscode.TreeDataProvider<vscode.TreeItem> {
  private readonly changes = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changes.event;

  constructor(private readonly registry: SessionRegistry) {}

  refresh(): void {
    this.changes.fire();
  }

  getTreeItem(item: vscode.TreeItem): vscode.TreeItem {
    return item;
  }

  getChildren(): vscode.TreeItem[] {
    const items = this.registry.all().flatMap((session) => {
      const operations: vscode.TreeItem[] = [];
      if (session.graphState === "building") {
        operations.push(operation("Building graph", session.root, "sync~spin"));
      }
      if (session.watch) operations.push(operation("Watching", session.root, "eye"));
      return operations;
    });
    return items.length > 0 ? items : [operation("No active operations", "", "check")];
  }
}

function operation(label: string, description: string, icon: string): vscode.TreeItem {
  const item = new vscode.TreeItem(label);
  item.description = description;
  item.iconPath = new vscode.ThemeIcon(icon);
  return item;
}
