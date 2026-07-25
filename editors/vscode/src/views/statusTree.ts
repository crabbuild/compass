import * as vscode from "vscode";
import type { CompassDiscovery } from "../cli/discovery";
import type { SessionRegistry } from "../workspace/sessionRegistry";
import { buildRepositoryTree, type TreeNode } from "./treeModel";

export class StatusTree implements vscode.TreeDataProvider<TreeNode> {
  private readonly changes = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changes.event;

  constructor(
    private readonly registry: SessionRegistry,
    private readonly discovery: CompassDiscovery
  ) {}

  refresh(): void {
    this.changes.fire();
  }

  getTreeItem(node: TreeNode): vscode.TreeItem {
    const state = node.children?.length
      ? node.expanded
        ? vscode.TreeItemCollapsibleState.Expanded
        : vscode.TreeItemCollapsibleState.Collapsed
      : vscode.TreeItemCollapsibleState.None;
    const item = new vscode.TreeItem(node.label, state);
    item.id = node.id;
    if (node.description !== undefined) item.description = node.description;
    if (node.tooltip !== undefined) item.tooltip = node.tooltip;
    item.iconPath = new vscode.ThemeIcon(node.icon);
    if (node.command) {
      item.command = {
        command: node.command,
        title: node.label
      };
      if (node.commandArguments !== undefined) {
        item.command.arguments = node.commandArguments;
      }
    }
    return item;
  }

  getChildren(node?: TreeNode): TreeNode[] {
    if (node) return node.children ?? [];
    return buildRepositoryTree(this.discovery, this.registry.all());
  }
}
