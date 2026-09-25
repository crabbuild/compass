import { describe, expect, it } from "vitest";
import type { GraphNode, GraphViewModel } from "../contracts/graph";
import { CommunityHierarchyViewSchema } from "../contracts/hierarchy";
import {
  communityFacts,
  hierarchyCommunityNames,
  withCommunityNames
} from "./communityFacts";

function bubble(id: string, community: number, label = `Community ${community}`): GraphNode {
  return { id, label, kind: "community", community, memberCount: 120 };
}

function graph(nodes: GraphNode[], edges: GraphViewModel["edges"] = []): GraphViewModel {
  return {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes: nodes.length, edges: edges.length, communities: 2, aggregated: true },
    nodes,
    edges,
    communities: [
      { id: 0, label: "Community 0", color: "#4E79A7", hidden: false },
      { id: 1, label: "Community 1", color: "#F28E2B", hidden: false }
    ],
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
        memberCount: 240,
        groups: [{
          index: 0,
          id: "h0-0000000000000001",
          signature: "0000000000000001",
          label: "src/runtime",
          labelRule: "dominantDirectory" as const,
          labelGeneric: false,
          memberCount: 240,
          childIndices: [0, 1],
          cohesion: 0.5,
          conductance: 0.25,
          boundaryKinds: { route: 3 },
          detailAvailable: false
        }],
        model: {
          schema: "compass.viewer.graph/1" as const,
          title: "Fixture",
          stats: { nodes: 1, edges: 0, communities: 1, aggregated: true },
          nodes: [{
            id: "h0-0000000000000001",
            label: "src/runtime",
            kind: "community",
            community: 0,
            memberCount: 240
          }],
          edges: [],
          communities: [],
          hyperedges: []
        }
      },
      {
        level: 1,
        merge: "relationship" as const,
        resolution: 1,
        groupCount: 2,
        memberCount: 240,
        groups: [
          {
            index: 0,
            id: "h1-0000000000000001",
            signature: "0000000000000001",
            community: 0,
            label: "src/runtime/a",
            labelRule: "dominantDirectory" as const,
            labelGeneric: false,
            memberCount: 120,
            childIndices: [],
            cohesion: 1,
            conductance: 0,
            boundaryKinds: {},
            detailAvailable: true
          },
          {
            index: 1,
            id: "h1-0000000000000002",
            signature: "0000000000000002",
            community: 1,
            label: "src/runtime/b",
            labelRule: "dominantDirectory" as const,
            labelGeneric: false,
            memberCount: 120,
            childIndices: [],
            cohesion: 1,
            conductance: 0,
            boundaryKinds: {},
            detailAvailable: true
          }
        ],
        model: {
          schema: "compass.viewer.graph/1" as const,
          title: "Fixture",
          stats: { nodes: 2, edges: 1, communities: 2, aggregated: true },
          nodes: [
            {
              id: "h1-0000000000000001",
              label: "src/runtime/a",
              kind: "community",
              community: 0,
              memberCount: 120
            },
            {
              id: "h1-0000000000000002",
              label: "src/runtime/b",
              kind: "community",
              community: 1,
              memberCount: 120
            }
          ],
          edges: [{
            id: "level-edge",
            source: "h1-0000000000000001",
            target: "h1-0000000000000002",
            relation: "3 cross-community edges",
            weight: 3
          }],
          communities: [],
          hyperedges: []
        }
      }
    ]
  });
}

describe("hierarchy community names", () => {
  it("names every published community from the finest level", () => {
    const names = hierarchyCommunityNames(hierarchy());
    expect(names.get(0)).toBe("src/runtime/a");
    expect(names.get(1)).toBe("src/runtime/b");
    expect(hierarchyCommunityNames(undefined).size).toBe(0);
  });

  it("replaces placeholders and keeps the labels an export published", () => {
    const view = hierarchy();
    const names = hierarchyCommunityNames(view);
    const named = withCommunityNames(
      graph([
        { id: "community:0", label: "Community 0", kind: "community", community: 0 },
        { id: "community:1", label: "Curated", kind: "community", community: 1 },
        { id: "symbol", label: "run()", community: 0, communityName: "Community 0" },
        { id: "named", label: "helper()", community: 1, communityName: "Own words" }
      ]),
      names
    );
    expect(named.nodes[0]?.label).toBe("src/runtime/a");
    expect(named.nodes[1]?.label).toBe("Curated");
    expect(named.nodes[2]?.label).toBe("run()");
    expect(named.nodes[2]?.communityName).toBe("src/runtime/a");
    expect(named.nodes[3]?.communityName).toBe("Own words");
    expect(named.communities.map((community) => community.label))
      .toEqual(["src/runtime/a", "src/runtime/b"]);
  });

  it("returns the same model when nothing names a community", () => {
    const model = graph([bubble("community:0", 0)]);
    expect(withCommunityNames(model, new Map())).toBe(model);
  });
});

describe("community facts", () => {
  it("reads a level group's own evidence", () => {
    const view = hierarchy();
    const model = view.levels[0]!.model!;
    const node = model.nodes[0]!;
    const facts = communityFacts(view, model, node, { communityKeyed: false });
    expect(facts).toMatchObject({
      label: "src/runtime",
      symbols: 240,
      groupId: "h0-0000000000000001",
      level: 0,
      merge: "locationAffinity",
      childGroups: 2,
      cohesion: 0.5,
      conductance: 0.25
    });
    expect(facts?.communityId).toBeUndefined();
    expect(facts?.boundaryKinds).toEqual([["route", 3]]);
  });

  it("couples a narrowed group through the level projection it came from", () => {
    const view = hierarchy();
    const finest = view.levels[1]!.model!;
    const facts = communityFacts(view, finest, finest.nodes[0]!, { communityKeyed: false });
    expect(facts?.couplings).toEqual([{ label: "src/runtime/b", edges: 3 }]);
    expect(facts?.couplingCount).toBe(3);
    expect(facts?.couplingsComplete).toBe(true);
  });

  it("pairs a drawn community with the finest level's group", () => {
    const view = hierarchy();
    const model = graph([bubble("community:1", 1, "src/runtime/b")]);
    const facts = communityFacts(view, model, model.nodes[0]!);
    expect(facts).toMatchObject({
      label: "src/runtime/b",
      communityId: 1,
      level: 1,
      childGroups: 0
    });
  });

  it("does not read a level's group number as a published community", () => {
    // Level 0's node is community 0 by group index; only the pairing the finest
    // level publishes names a community, so nothing may be opened from it.
    const view = hierarchy();
    const model = view.levels[0]!.model!;
    const facts = communityFacts(view, model, model.nodes[0]!, { communityKeyed: false });
    expect(facts?.groupId).toBe("h0-0000000000000001");
    expect(facts?.communityId).toBeUndefined();
  });

  it("reports a plain aggregated bubble without a hierarchy", () => {
    const model = graph([bubble("community:0", 0, "Core")]);
    const facts = communityFacts(undefined, model, model.nodes[0]!);
    expect(facts).toMatchObject({ label: "Core", symbols: 120, couplingCount: 0 });
    expect(facts?.groupId).toBeUndefined();
  });

  it("ignores a symbol", () => {
    const model = graph([{ id: "run", label: "run()", community: 0 }]);
    model.stats.aggregated = false;
    expect(communityFacts(undefined, model, model.nodes[0]!)).toBeUndefined();
  });
});
