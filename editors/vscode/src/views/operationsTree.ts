import * as vscode from "vscode";
import type { SessionRegistry } from "../workspace/sessionRegistry";
import { treeItemFromNode } from "./treeItem";
import { buildOperationsTree, type TreeNode } from "./treeModel";

export class OperationsTree implements vscode.TreeDataProvider<TreeNode> {
  private readonly changes = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changes.event;

  constructor(private readonly registry: SessionRegistry) {}

  refresh(): void {
    this.changes.fire();
  }

  getTreeItem(node: TreeNode): vscode.TreeItem {
    return treeItemFromNode(node);
  }

  getChildren(node?: TreeNode): TreeNode[] {
    if (node) return node.children ?? [];
    return buildOperationsTree(this.registry.all());
  }
}
