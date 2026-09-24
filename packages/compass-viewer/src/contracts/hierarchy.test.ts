import { describe, expect, it } from "vitest";
import { CommunityHierarchyViewSchema } from "./hierarchy";

function levelModel(id: string) {
  return {
    schema: "compass.viewer.graph/1" as const,
    title: "Fixture",
    stats: { nodes: 1, edges: 0, communities: 1, aggregated: true },
    nodes: [{
      id, label: id, community: Number(id), memberCount: 2
    }],
    edges: [],
    communities: [],
    hyperedges: []
  };
}

function view() {
  return {
    schema: "compass.viewer.hierarchy/1" as const,
    budgetIdentity: "community-hierarchy-budget/v1",
    mergePolicy: "relationship-then-location-affinity/v1",
    rootTarget: 24,
    levelTarget: 300,
    maxLevels: 4,
    budgetSatisfied: true,
    finestCommunityCount: 1,
    finestSignature: `sha256:${"0".repeat(64)}`,
    boundaryKinds: ["route"],
    levels: [{
      level: 0,
      merge: "locationAffinity" as const,
      groupCount: 1,
      memberCount: 2,
      groups: [{
        index: 0,
        community: 0,
        label: "src",
        labelRule: "dominantDirectory" as const,
        labelGeneric: false,
        memberCount: 2,
        childIndices: [],
        cohesion: 0.5,
        conductance: 0.25,
        boundaryKinds: { route: 1 },
        detailAvailable: false
      }],
      model: levelModel("0")
    }]
  };
}

describe("community hierarchy view contract", () => {
  it("accepts a published level with its evidence", () => {
    const parsed = CommunityHierarchyViewSchema.parse(view());
    const level = parsed.levels[0];
    const group = level?.groups[0];
    expect(level?.merge).toBe("locationAffinity");
    expect(group?.labelRule).toBe("dominantDirectory");
    expect(parsed.budgetSatisfied).toBe(true);
  });

  it("accepts a level the export could not draw", () => {
    const payload = view();
    const level = payload.levels[0] as {
      model?: unknown;
      resolution?: unknown;
    };
    delete level.model;
    delete level.resolution;
    const parsed = CommunityHierarchyViewSchema.parse(payload);
    expect(parsed.levels[0]?.model).toBeUndefined();
  });

  it("rejects an unknown schema, merge rule, or label rule", () => {
    expect(CommunityHierarchyViewSchema.safeParse({
      ...view(), schema: "compass.viewer.hierarchy/2"
    }).success).toBe(false);
    const merge = view();
    const mergeLevel = merge.levels[0] as { merge: unknown };
    mergeLevel.merge = "clustering";
    expect(CommunityHierarchyViewSchema.safeParse(merge).success).toBe(false);
    const label = view();
    const labelGroup = label.levels[0]?.groups[0] as { labelRule: unknown };
    labelGroup.labelRule = "directory";
    expect(CommunityHierarchyViewSchema.safeParse(label).success).toBe(false);
  });

  it("requires labels to say whether they are generic", () => {
    const payload = view();
    const group = payload.levels[0]?.groups[0] as { labelGeneric?: unknown };
    delete group.labelGeneric;
    expect(CommunityHierarchyViewSchema.safeParse(payload).success).toBe(false);
  });
});
