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

export function buildWorkspaceTree(
  discovery: CompassDiscovery,
  sessions: readonly SessionTreeSnapshot[]
): TreeNode[] {
  if (sessions.length === 0) {
    return [
      actionNode(
        "workspace:open-folder",
        "Open a repository folder",
        "folder-opened",
        "vscode.openFolder",
        "Open a folder to use Compass"
      )
    ];
  }

  const attention = cliAttentionNodes(discovery, sessions);
  const nodes = [...attention, ...repositoryStatusNodes(sessions)];
  if (attention.length > 0) {
    return nodes;
  }

  const active = activeOperationGroup(sessions);
  if (active.length > 0) {
    nodes.push({
      id: "workspace:active",
      label: "Active operations",
      description: String(active.length),
      icon: "pulse",
      expanded: true,
      children: active
    });
  }

  const missing = sessions.filter((session) => session.graphState === "not-materialized");
  const failed = sessions.filter((session) => session.graphState === "failed");
  const hasMissing = missing.length > 0;
  const hasGraph = sessions.some((session) => session.graphState === "available");
  const hasFailed = failed.length > 0;
  const hasWatch = sessions.some((session) => Boolean(session.watch));

  if (hasMissing) {
    nodes.push(actionNode(
      "workspace:initialize",
      "Initialize repository",
      "rocket",
      "compass.initialize",
      "Build the first Compass graph",
      missing.length === 1 ? [missing[0]?.id] : undefined
    ));
  }
  if (hasFailed) {
    nodes.push(actionNode(
      "workspace:retry",
      "Retry graph build",
      "refresh",
      "compass.update",
      "Retry a failed Compass graph build",
      failed.length === 1 ? [failed[0]?.id] : undefined
    ));
  }

  const explore: TreeNode[] = [];
  if (hasGraph) {
    explore.push(
      actionNode(
        "workspace:code-graph",
        "Code graph",
        "type-hierarchy",
        "compass.openGraph",
        "Explore the current repository graph"
      ),
      actionNode(
        "workspace:architecture",
        "Architecture flow",
        "circuit-board",
        "compass.openArchitecture",
        "Read the codebase architecture flow"
      ),
      actionNode(
        "workspace:call-graph",
        "Call graph from cursor",
        "references",
        "compass.openCallGraph",
        "Trace callers and callees for the active function"
      ),
      actionNode(
        "workspace:query",
        "Ask codebase",
        "search",
        "compass.openQuery",
        "Ask a natural-language or CompassQL question"
      )
    );
  }
  explore.push(actionNode(
    "workspace:evolution",
    "Codebase evolution",
    "history",
    "compass.openHistory",
    "Browse Git commits and revision graphs"
  ));
  nodes.push({
    id: "workspace:explore",
    label: "Explore",
    icon: "compass",
    expanded: true,
    children: explore
  });

  const maintain: TreeNode[] = [];
  if (hasGraph && !hasFailed) {
    maintain.push(actionNode(
      "workspace:update",
      "Update graph",
      "refresh",
      "compass.update",
      "Refresh changed code relationships"
    ));
  }
  if (hasGraph || hasWatch) {
    maintain.push(actionNode(
      "workspace:watch",
      sessions.length === 1 && hasWatch ? "Stop watching" : "Watch for changes",
      sessions.length === 1 && hasWatch ? "debug-stop" : "eye",
      "compass.toggleWatch",
      sessions.length === 1
        ? hasWatch
          ? "Stop watching this repository"
          : "Keep the graph current as files change"
        : "Choose a repository to start or stop watching"
    ));
  }
  if (maintain.length > 0) {
    nodes.push({
      id: "workspace:maintain",
      label: "Maintain",
      icon: "tools",
      children: maintain
    });
  }
  return nodes;
}

function cliAttentionNodes(
  discovery: CompassDiscovery,
  sessions: readonly SessionTreeSnapshot[]
): TreeNode[] {
  if (discovery.kind === "missing") {
    return [{
      id: "cli-setup",
      label: "Compass CLI needs attention",
      description: "Not found",
      tooltip: "Compass was not found in the configured location or on PATH.",
      icon: "warning",
      command: "compass.selectCli"
    }];
  }
  const incompatible = sessions.find((session) => session.capabilityError);
  if (incompatible) {
    return [{
      id: "cli-incompatible",
      label: "Compass CLI needs attention",
      description: "Incompatible",
      tooltip: incompatible.capabilityError ?? "The Compass CLI is incompatible.",
      icon: "warning",
      command: "compass.selectCli"
    }];
  }
  return [];
}

function repositoryStatusNodes(
  sessions: readonly SessionTreeSnapshot[]
): TreeNode[] {
  return sessions.map((session) => ({
    id: `repository:${session.id}`,
    label: path.basename(session.root) || session.root,
    description: graphStateLabel(session.graphState),
    tooltip: session.root,
    icon: graphStateIcon(session.graphState)
  }));
}

function activeOperationGroup(
  sessions: readonly SessionTreeSnapshot[]
): TreeNode[] {
  const active: TreeNode[] = [];
  for (const session of sessions) {
    const repositoryName = path.basename(session.root) || session.root;
    if (session.activeWriter) {
      active.push({
        id: `active-build:${session.id}`,
        label: "Building graph",
        description: repositoryName,
        tooltip: session.root,
        icon: "sync~spin"
      });
    }
    if (session.watch) {
      active.push({
        id: `active-watch:${session.id}`,
        label: "Watching for changes",
        description: repositoryName,
        tooltip: session.root,
        icon: "eye"
      });
    }
  }
  return active;
}

export function graphStateLabel(state: GraphState): string {
  if (state === "available") return "Graph ready";
  if (state === "not-materialized") return "Not initialized";
  if (state === "building") return "Building";
  return "Build failed";
}

export function graphStateIcon(state: GraphState): string {
  if (state === "available") return "pass";
  if (state === "building") return "sync~spin";
  if (state === "failed") return "error";
  return "circle-large-outline";
}
