import * as vscode from "vscode";
import type { CompassDiscovery } from "../cli/discovery";
import type { SessionRegistry } from "../workspace/sessionRegistry";
import { treeItemFromNode } from "./treeItem";
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
    return treeItemFromNode(node);
  }

  getChildren(node?: TreeNode): TreeNode[] {
    if (node) return node.children ?? [];
    return buildRepositoryTree(this.discovery, this.registry.all());
  }
}
