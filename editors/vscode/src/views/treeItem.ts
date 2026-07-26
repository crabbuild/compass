import * as vscode from "vscode";
import type { TreeNode } from "./treeModel";

export function treeItemFromNode(node: TreeNode): vscode.TreeItem {
  const collapsibleState = node.children?.length
    ? node.expanded
      ? vscode.TreeItemCollapsibleState.Expanded
      : vscode.TreeItemCollapsibleState.Collapsed
    : vscode.TreeItemCollapsibleState.None;
  const item = new vscode.TreeItem(node.label, collapsibleState);
  item.id = node.id;
  if (node.contextValue !== undefined) item.contextValue = node.contextValue;
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
