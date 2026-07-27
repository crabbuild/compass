import { describe, expect, it } from "vitest";
import {
  buildWorkspaceTree,
  type SessionTreeSnapshot,
  type TreeNode
} from "./treeModel";

const discovery = {
  kind: "found" as const,
  executable: "/usr/local/bin/compass"
};

const available: SessionTreeSnapshot = {
  id: "repository-1",
  root: "/work/repo",
  graphState: "available",
  capabilityError: undefined
};

describe("buildWorkspaceTree", () => {
  it("shows repository status and each healthy workflow exactly once", () => {
    const nodes = buildWorkspaceTree(discovery, [available]);

    expect(nodes.map((node) => node.label)).toEqual([
      "repo",
      "Explore"
    ]);
    expect(nodes[0]).toMatchObject({
      description: "Graph ready",
      tooltip: "/work/repo",
      contextValue: "compass.repository.ready",
      repositoryId: "repository-1"
    });
    expect(nodes[0]?.children).toBeUndefined();
    expect(nodes[1]?.expanded).toBe(true);
    expect(nodes[1]?.children?.map((node) => [node.label, node.command])).toEqual([
      ["Code graph", "compass.openGraph"],
      ["Architecture flow", "compass.openArchitecture"],
      ["Call graph from cursor", "compass.openCallGraphGuide"],
      ["Ask codebase", "compass.openQuery"],
      ["Codebase evolution", "compass.openHistory"]
    ]);
    expect(new Set(commands(nodes)).size).toBe(commands(nodes).length);
  });

  it("keeps active operations status-only and moves stop control to Maintain", () => {
    const nodes = buildWorkspaceTree(discovery, [{
      ...available,
      activeWriter: { operationId: "build-1" },
      watch: { operationId: "watch-1" }
    }]);

    expect(nodes.map((node) => node.label)).toEqual([
      "repo",
      "Active operations",
      "Explore"
    ]);
    expect(nodes[1]).toMatchObject({ description: "2", expanded: true });
    expect(nodes[0]?.contextValue).toBe("compass.repository.watching");
    expect(nodes[1]?.children?.map((node) => ({
      label: node.label,
      description: node.description,
      command: node.command
    }))).toEqual([
      { label: "Building graph", description: "repo", command: undefined },
      { label: "Watching for changes", description: "repo", command: undefined }
    ]);
    expect(new Set(commands(nodes)).size).toBe(commands(nodes).length);
  });

  it("offers one initialization action before the first graph", () => {
    const nodes = buildWorkspaceTree(discovery, [{
      ...available,
      graphState: "not-materialized"
    }]);

    expect(nodes.map((node) => node.label)).toEqual([
      "repo",
      "Initialize repository",
      "Explore"
    ]);
    expect(nodes[0]?.description).toBe("Not initialized");
    expect(nodes[1]?.command).toBe("compass.initialize");
    expect(nodes[2]?.children?.map((node) => node.label)).toEqual([
      "Codebase evolution"
    ]);
    expect(new Set(commands(nodes)).size).toBe(commands(nodes).length);
  });

  it("offers one targeted retry after a failed graph build", () => {
    const nodes = buildWorkspaceTree(discovery, [{
      ...available,
      graphState: "failed"
    }]);

    expect(nodes.map((node) => node.label)).toEqual([
      "repo",
      "Retry graph build",
      "Explore"
    ]);
    expect(nodes[0]?.description).toBe("Build failed");
    expect(nodes[1]?.command).toBe("compass.update");
    expect(commands(nodes).filter((command) => command === "compass.update")).toHaveLength(1);
  });

  it("focuses the tree on setup when the CLI is missing or incompatible", () => {
    const missing = buildWorkspaceTree(
      { kind: "missing", searched: ["/usr/bin/compass"] },
      [available]
    );
    expect(missing.map((node) => node.label)).toEqual([
      "Compass CLI needs attention",
      "repo"
    ]);
    expect(missing[0]).toMatchObject({
      description: "Not found",
      command: "compass.selectCli"
    });

    const incompatible = buildWorkspaceTree(discovery, [{
      ...available,
      capabilityError: "capability contract is too old"
    }]);
    expect(incompatible.map((node) => node.label)).toEqual([
      "Compass CLI needs attention",
      "repo"
    ]);
    expect(incompatible[0]).toMatchObject({
      description: "Incompatible",
      tooltip: "capability contract is too old"
    });
  });

  it("offers a quiet native action when no repository is open", () => {
    const nodes = buildWorkspaceTree(discovery, []);

    expect(nodes).toEqual([expect.objectContaining({
      label: "Open a repository folder",
      icon: "folder-opened",
      command: "vscode.openFolder"
    })]);
  });

  it("keeps multi-root status explicit without repeating global workflows", () => {
    const nodes = buildWorkspaceTree(discovery, [
      available,
      {
        ...available,
        id: "repository-2",
        root: "/work/other",
        graphState: "failed"
      }
    ]);

    expect(nodes.filter((node) => node.id.startsWith("repository:")).map((node) => node.label))
      .toEqual(["repo", "other"]);
    expect(nodes.filter((node) => node.label === "Explore")).toHaveLength(1);
    expect(new Set(commands(nodes)).size).toBe(commands(nodes).length);
  });
});

function commands(nodes: readonly TreeNode[]): string[] {
  return nodes.flatMap((node) => [
    ...(node.command ? [node.command] : []),
    ...commands(node.children ?? [])
  ]);
}
