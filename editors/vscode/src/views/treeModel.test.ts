import { describe, expect, it } from "vitest";
import { buildRepositoryTree, type SessionTreeSnapshot } from "./treeModel";

const available: SessionTreeSnapshot = {
  id: "repository-1",
  root: "/work/repo",
  graphState: "available",
  capabilityError: undefined
};

describe("buildRepositoryTree", () => {
  it("hides a healthy CLI and exposes repository graph actions", () => {
    const nodes = buildRepositoryTree(
      { kind: "found", executable: "/usr/local/bin/compass" },
      [available]
    );

    expect(nodes).toHaveLength(1);
    expect(nodes[0]).toMatchObject({
      label: "repo",
      description: "Graph available",
      tooltip: "/work/repo",
      expanded: true
    });
    expect(nodes[0]?.children?.map((node) => node.command)).toEqual([
      "compass.openGraph",
      "compass.openHistory"
    ]);
    expect(nodes[0]?.children?.[0]?.commandArguments).toEqual(["repository-1"]);
  });

  it("shows CLI setup only when discovery or compatibility needs attention", () => {
    const missing = buildRepositoryTree(
      { kind: "missing", searched: ["/usr/bin/compass"] },
      [{ ...available, graphState: "not-materialized" }]
    );
    expect(missing[0]).toMatchObject({
      label: "Compass CLI needs attention",
      description: "Not found",
      command: "compass.selectCli"
    });
    expect(missing[1]?.children?.[0]?.command).toBe("compass.initialize");

    const incompatible = buildRepositoryTree(
      { kind: "found", executable: "/usr/local/bin/compass" },
      [{ ...available, capabilityError: "capability contract is too old" }]
    );
    expect(incompatible[0]).toMatchObject({
      description: "Incompatible",
      tooltip: "capability contract is too old",
      command: "compass.selectCli"
    });
  });

  it("offers a targeted retry after a failed build", () => {
    const nodes = buildRepositoryTree(
      { kind: "found", executable: "/usr/local/bin/compass" },
      [{ ...available, graphState: "failed" }]
    );

    expect(nodes[0]?.description).toBe("Build failed");
    expect(nodes[0]?.children?.[0]).toMatchObject({
      label: "Update graph",
      command: "compass.update",
      commandArguments: ["repository-1"]
    });
  });
});
