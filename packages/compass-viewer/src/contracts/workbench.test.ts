import { describe, expect, it } from "vitest";
import { WorkbenchModelSchema } from "./workbench";

const graph = {
  schema: "compass.viewer.graph/1",
  title: "Fixture",
  stats: { nodes: 1, edges: 0, communities: 1, aggregated: false },
  nodes: [{ id: "run", label: "run", community: 0, depth: 0, root: true }],
  edges: [],
  communities: [{ id: 0, label: "Core", color: "#4e79a7" }],
  hyperedges: []
};

describe("WorkbenchModelSchema", () => {
  it("accepts ordered mixed graph lenses", () => {
    const model = WorkbenchModelSchema.parse({
      schema: "compass.viewer.workbench/1",
      title: "Fixture",
      graphIdentity: "sha256:fixture",
      defaultView: "code",
      views: [
        {
          id: "code",
          title: "Code graph",
          description: "Repository structure",
          coverage: {
            status: "complete",
            truncated: false,
            nodes: 1,
            edges: 0,
            limitations: []
          },
          kind: "code",
          model: graph,
          communityDetails: {}
        },
        {
          id: "affected-run",
          title: "Affected · run",
          description: "Reverse dependencies",
          coverage: {
            status: "partial",
            truncated: true,
            nodes: 1,
            edges: 0,
            limitations: ["Bound reached"]
          },
          kind: "affected",
          root: "run",
          relations: ["calls"],
          depth: 2,
          model: graph
        }
      ]
    });
    expect(model.views.map((view) => view.id)).toEqual(["code", "affected-run"]);
    expect(model.views[1]?.coverage.status).toBe("partial");
    expect(model.views[0]?.kind === "code" && model.views[0].model.nodes[0]).toMatchObject({
      depth: 0,
      root: true
    });
  });

  it("rejects duplicate ids and a missing default view", () => {
    const view = {
      id: "code",
      title: "Code graph",
      description: "Repository structure",
      coverage: { status: "complete", truncated: false, nodes: 1, edges: 0 },
      kind: "code",
      model: graph
    };
    expect(() => WorkbenchModelSchema.parse({
      schema: "compass.viewer.workbench/1",
      title: "Fixture",
      graphIdentity: "sha256:fixture",
      defaultView: "missing",
      views: [view, view]
    })).toThrow();
  });
});
