import path from "node:path";
import type { CompassDiscovery } from "../cli/discovery";
import type { GraphState } from "../workspace/repositorySession";

export type TreeNode = {
  id: string;
  label: string;
  description?: string;
  tooltip?: string;
  icon: string;
  command?: string;
  commandArguments?: unknown[];
  expanded?: boolean;
  children?: TreeNode[];
};

export type SessionTreeSnapshot = {
  id: string;
  root: string;
  graphState: GraphState;
  capabilityError: string | undefined;
  activeWriter?: unknown;
  watch?: unknown;
};

export function actionNode(
  id: string,
  label: string,
  icon: string,
  command: string,
  description?: string,
  commandArguments?: unknown[]
): TreeNode {
  const node: TreeNode = {
    id,
    label,
    icon,
    command
  };
  if (description !== undefined) node.description = description;
  if (commandArguments !== undefined) node.commandArguments = commandArguments;
  return node;
}

export function buildRepositoryTree(
  discovery: CompassDiscovery,
  sessions: readonly SessionTreeSnapshot[]
): TreeNode[] {
  const nodes: TreeNode[] = [];
  const incompatible = sessions.find((session) => session.capabilityError);
  if (discovery.kind === "missing") {
    nodes.push({
      id: "cli-setup",
      label: "Compass CLI needs attention",
      description: "Not found",
      tooltip: "Compass was not found in the configured location or on PATH.",
      icon: "warning",
      command: "compass.selectCli"
    });
  } else if (incompatible) {
    nodes.push({
      id: "cli-incompatible",
      label: "Compass CLI needs attention",
      description: "Incompatible",
      tooltip: incompatible.capabilityError ?? "The Compass CLI is incompatible.",
      icon: "warning",
      command: "compass.selectCli"
    });
  }

  for (const session of sessions) {
    const repositoryName = path.basename(session.root) || session.root;
    const commandArguments = [session.id];
    const children = repositoryActions(session, commandArguments);
    nodes.push({
      id: `repository:${session.id}`,
      label: repositoryName,
      description: graphStateLabel(session.graphState),
      tooltip: session.root,
      icon: graphStateIcon(session.graphState),
      expanded: children.length > 0,
      children
    });
  }
  return nodes;
}

function repositoryActions(
  session: SessionTreeSnapshot,
  commandArguments: unknown[]
): TreeNode[] {
  if (session.graphState === "available") {
    return [
      actionNode(
        `open-graph:${session.id}`,
        "Open graph",
        "type-hierarchy",
        "compass.openGraph",
        "Explore the current code graph",
        commandArguments
      ),
      actionNode(
        `open-history:${session.id}`,
        "Codebase evolution",
        "history",
        "compass.openHistory",
        "Browse Git commits and revision graphs",
        commandArguments
      )
    ];
  }
  if (session.graphState === "not-materialized") {
    return [
      actionNode(
        `initialize:${session.id}`,
        "Initialize repository",
        "rocket",
        "compass.initialize",
        "Build the first Compass graph",
        commandArguments
      )
    ];
  }
  if (session.graphState === "failed") {
    return [
      actionNode(
        `retry-update:${session.id}`,
        "Update graph",
        "refresh",
        "compass.update",
        "Retry the failed graph build",
        commandArguments
      )
    ];
  }
  return [];
}

export function graphStateLabel(state: GraphState): string {
  if (state === "available") return "Graph available";
  if (state === "not-materialized") return "Not materialized";
  if (state === "building") return "Building graph";
  return "Build failed";
}

export function graphStateIcon(state: GraphState): string {
  if (state === "available") return "pass";
  if (state === "building") return "sync~spin";
  if (state === "failed") return "error";
  return "circle-large-outline";
}
