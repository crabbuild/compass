import { describe, expect, it } from "vitest";
import { CommunityHierarchyViewSchema } from "../contracts/hierarchy";
import {
  ROOT_SCOPE,
  descendModel,
  groupAt,
  hierarchyNavigable,
  hierarchyTrail,
  scopeLevel,
  scopeModel
} from "./hierarchyLevels";

function model(ids: string[], edges: Array<[string, string]> = []) {
  return {
    schema: "compass.viewer.graph/1" as const,
    title: "Fixture",
    stats: { nodes: ids.length, edges: edges.length, communities: ids.length, aggregated: true },
    nodes: ids.map((id) => ({ id, label: id, community: Number(id), memberCount: 2 })),
    edges: edges.map(([source, target], position) => ({
      id: `edge-${position}`, source, target, relation: "calls"
    })),
    communities: ids.map((id) => ({
      id: Number(id), label: `group ${id}`, color: "#6688aa", hidden: false
    })),
    hyperedges: []
  };
}

function hierarchy() {
  return CommunityHierarchyViewSchema.parse({
    schema: "compass.viewer.hierarchy/1",
    budgetIdentity: "community-hierarchy-budget/v1",
    mergePolicy: "relationship-then-location-affinity/v1",
    rootTarget: 24,
    levelTarget: 300,
    maxLevels: 4,
    budgetSatisfied: true,
    finestCommunityCount: 2,
    finestSignature: `sha256:${"0".repeat(64)}`,
    boundaryKinds: ["route"],
    levels: [
      {
        level: 0,
        merge: "locationAffinity" as const,
        groupCount: 1,
        memberCount: 4,
        groups: [{
          index: 0,
          label: "src",
          labelRule: "dominantDirectory" as const,
          labelGeneric: false,
          memberCount: 4,
          childIndices: [0, 1],
          cohesion: 0,
          conductance: 0,
          boundaryKinds: {},
          detailAvailable: false
        }],
        model: model(["0"])
      },
      {
        level: 1,
        merge: "relationship" as const,
        resolution: 1,
        groupCount: 2,
        memberCount: 4,
        groups: [
          {
            index: 0,
            community: 0,
            label: "src/one",
            labelRule: "dominantDirectory" as const,
            labelGeneric: false,
            memberCount: 2,
            childIndices: [],
            cohesion: 1,
            conductance: 0,
            boundaryKinds: {},
            detailAvailable: true
          },
          {
            index: 1,
            community: 1,
            label: "Community 1",
            labelRule: "communityId" as const,
            labelGeneric: true,
            memberCount: 2,
            childIndices: [],
            cohesion: 0,
            conductance: 0.5,
            boundaryKinds: { route: 1 },
            detailAvailable: true
          }
        ],
        model: model(["0", "1"], [["0", "1"]])
      }
    ]
  });
}

describe("hierarchy level scopes", () => {
  it("draws the level the scope names", () => {
    const view = hierarchy();
    expect(scopeModel(view, ROOT_SCOPE)?.nodes).toHaveLength(1);
    expect(scopeLevel(view, { kind: "level", level: 1 })?.groups).toHaveLength(2);
    expect(scopeModel(view, { kind: "symbols" })).toBeUndefined();
  });

  it("narrows to a group's children without redrawing the level", () => {
    const view = hierarchy();
    const child = descendModel(view, 0, 0);
    expect(child?.nodes.map((node) => node.id)).toEqual(["0", "1"]);
    expect(child?.edges.map((edge) => edge.id)).toEqual(["edge-0"]);
    expect(child?.communities.map((community) => community.id)).toEqual([0, 1]);
    expect(child?.stats).toMatchObject({ nodes: 2, edges: 1, communities: 2 });
    expect(descendModel(view, 1, 0)).toBeUndefined();
  });

  it("publishes only the groups a narrowed scope draws", () => {
    const view = hierarchy();
    const root = view.levels[0];
    const rootGroup = root?.groups[0];
    if (rootGroup) {
      rootGroup.childIndices = [0];
    }
    const child = descendModel(view, 0, 0);
    expect(child?.nodes.map((node) => node.id)).toEqual(["0"]);
    expect(child?.communities.map((community) => community.id)).toEqual([0]);
    expect(child?.stats).toMatchObject({ nodes: 1, edges: 0, communities: 1 });
  });

  it("drops edges that leave the narrowed group", () => {
    const view = hierarchy();
    const level = view.levels[1];
    if (level?.model) {
      level.model.edges.push({ id: "edge-out", source: "0", target: "9", relation: "calls" });
    }
    const child = descendModel(view, 0, 0);
    expect(child?.edges.map((edge) => edge.id)).toEqual(["edge-0"]);
  });

  it("names the group a scope narrowed to and the trail above it", () => {
    const view = hierarchy();
    expect(groupAt(view, ROOT_SCOPE, 0)?.label).toBe("src");
    expect(groupAt(view, ROOT_SCOPE, 4)).toBeUndefined();
    expect(hierarchyTrail(view, ROOT_SCOPE, [])).toEqual(["Repository"]);
    expect(hierarchyTrail(view, { kind: "level", level: 1 }, [
      { level: 0, groupIndex: 0 }
    ])).toEqual(["Repository", "src"]);
    expect(hierarchyTrail(view, { kind: "symbols" }, [])).toEqual(["Symbols"]);
  });

  it("reports whether every published level can be drawn", () => {
    const view = hierarchy();
    expect(hierarchyNavigable(view)).toBe(true);
    const withoutModel = CommunityHierarchyViewSchema.parse({
      ...view,
      levels: view.levels.map((level) => ({ ...level, model: undefined }))
    });
    expect(hierarchyNavigable(withoutModel)).toBe(false);
  });
});
