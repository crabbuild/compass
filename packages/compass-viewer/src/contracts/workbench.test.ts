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
  it("accepts a code view that carries the published hierarchy", () => {
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
            hierarchyLevels: 2,
            limitations: []
          },
          kind: "code",
          model: graph,
          communityDetails: {},
          hierarchy: {
            schema: "compass.viewer.hierarchy/1",
            budgetIdentity: "community-hierarchy-budget/v1",
            mergePolicy: "relationship-then-location-affinity/v1",
            rootTarget: 24,
            levelTarget: 300,
            maxLevels: 4,
            budgetSatisfied: true,
            finestCommunityCount: 1,
            finestSignature: `sha256:${"0".repeat(64)}`,
            boundaryKinds: [],
            levels: [{
              level: 0,
              merge: "locationAffinity" as const,
              groupCount: 1,
              memberCount: 1,
              groups: [{
                index: 0,
                id: "h0-0000000000000001",
                signature: "0000000000000001",
                label: "src",
                labelRule: "dominantDirectory" as const,
                labelGeneric: false,
                memberCount: 1,
                childIndices: [],
                cohesion: 1,
                conductance: 0,
                boundaryKinds: {},
                detailAvailable: false
              }]
            }]
          }
        }
      ]
    });
    const view = model.views[0];
    expect(view?.kind === "code" ? view.hierarchy?.levels.length : undefined).toBe(1);
    expect(view?.coverage.hierarchyLevels).toBe(2);
  });

  it("rejects a code view whose hierarchy names another schema", () => {
    const result = WorkbenchModelSchema.safeParse({
      schema: "compass.viewer.workbench/1",
      title: "Fixture",
      graphIdentity: "sha256:fixture",
      defaultView: "code",
      views: [{
        id: "code",
        title: "Code graph",
        description: "Repository structure",
        coverage: { status: "complete", truncated: false, nodes: 1, edges: 0, limitations: [] },
        kind: "code",
        model: graph,
        communityDetails: {},
        hierarchy: { schema: "compass.viewer.hierarchy/2" }
      }]
    });
    expect(result.success).toBe(false);
  });

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
