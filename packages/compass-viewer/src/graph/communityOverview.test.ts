import { describe, expect, it } from "vitest";
import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";
import {
  COMMUNITY_DETAIL_NODE_BUDGET,
  COMMUNITY_OVERVIEW_NODE_THRESHOLD,
  communityDetailModel,
  communityNodeId,
  communityNodeSize,
  communityOverviewApplies,
  communityOverviewLabelLimit,
  communityOverviewLabelText,
  communityOverviewLabelledIds,
  communityOverviewModel
} from "./communityOverview";

function node(
  id: string,
  community: number,
  degree: number,
  extra: Partial<GraphNode> = {}
): GraphNode {
  return {
    id,
    label: id,
    kind: "function",
    community,
    degree,
    source: { file: `src/${community}.rs`, startLine: degree + 1 },
    ...extra
  };
}

function fixture(): GraphViewModel {
  const nodes: GraphNode[] = [
    node("auth-hub", 1, 12, { communityName: "Auth" }),
    node("auth-check", 1, 4),
    node("auth-token", 1, 2),
    node("db-hub", 2, 9, { communityName: "Database" }),
    node("db-query", 2, 3),
    node("ui-hub", 3, 5, { communityName: "Interface" })
  ];
  const edges: GraphEdge[] = [
    { id: "e1", source: "auth-hub", target: "auth-check", relation: "calls" },
    { id: "e2", source: "auth-check", target: "auth-token", relation: "calls" },
    { id: "e3", source: "auth-hub", target: "db-hub", relation: "calls" },
    { id: "e4", source: "auth-check", target: "db-hub", relation: "calls" },
    { id: "e5", source: "db-query", target: "ui-hub", relation: "imports" },
    { id: "e6", source: "db-hub", target: "db-query", relation: "calls" }
  ];
  return {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes: nodes.length, edges: edges.length, communities: 3, aggregated: false },
    nodes,
    edges,
    communities: [
      { id: 1, label: "Auth", color: "#4E79A7", hidden: false },
      { id: 2, label: "Community 2", color: "#F28E2B", hidden: false },
      { id: 3, label: "Interface", color: "#E15759", hidden: false }
    ],
    hyperedges: []
  };
}

function largeFixture(): GraphViewModel {
  const model = fixture();
  const nodes: GraphNode[] = [];
  for (let index = 0; index < COMMUNITY_OVERVIEW_NODE_THRESHOLD; index += 1) {
    const community = index % 3 + 1;
    nodes.push(node(`generated-${index}`, community, index % 7));
  }
  return { ...model, nodes: [...model.nodes, ...nodes] };
}

describe("communityOverviewApplies", () => {
  it("keeps a small repository on the symbol canvas", () => {
    expect(communityOverviewApplies(fixture())).toBe(false);
  });

  it("opens a large multi-community repository on the community overview", () => {
    expect(communityOverviewApplies(largeFixture())).toBe(true);
  });

  it("leaves an exported overview in place", () => {
    expect(communityOverviewApplies({
      ...largeFixture(),
      stats: { ...largeFixture().stats, aggregated: true }
    })).toBe(false);
  });

  it("keeps a single uncut community on the symbol canvas", () => {
    const model = largeFixture();
    expect(communityOverviewApplies({
      ...model,
      nodes: model.nodes.map((entry) => ({ ...entry, community: 0 }))
    })).toBe(false);
  });
});

describe("communityOverviewModel", () => {
  it("aggregates one labelled bubble per community with exact member counts", () => {
    const { model: overview } = communityOverviewModel(fixture());
    expect(overview.stats).toEqual({
      nodes: 3,
      edges: 2,
      communities: 3,
      aggregated: true
    });
    // Importance order: the two-community connector (`Database`) outranks the
    // three-member community because coupling and boundary content count.
    expect(overview.nodes.map((entry) => entry.id)).toEqual([
      communityNodeId(2),
      communityNodeId(1),
      communityNodeId(3)
    ]);
    expect(overview.nodes.map((entry) => entry.memberCount)).toEqual([2, 3, 1]);
    expect(overview.nodes.every((entry) => entry.kind === "community")).toBe(true);
    expect(overview.nodes.every((entry) => entry.detailAvailable === true)).toBe(true);
    // Community 2 has no declared label, so its most connected member names it.
    expect(overview.nodes[0]?.label).toBe("Database");
    expect(overview.nodes[1]?.label).toBe("Auth");
    expect(overview.nodes[1]?.color).toEqual({
      background: "#4E79A7",
      border: "#4E79A7"
    });
    expect(overview.communities.map((community) => community.id)).toEqual([1, 2, 3]);
  });

  it("states the exact relationship mix for every community pair", () => {
    const { model: overview, edgeCategories } = communityOverviewModel(fixture());
    expect(overview.edges).toEqual([
      {
        id: "community-edge:1:2",
        source: communityNodeId(1),
        target: communityNodeId(2),
        relation: "2 calls",
        weight: 2,
        confidence: "aggregated"
      },
      {
        id: "community-edge:2:3",
        source: communityNodeId(2),
        target: communityNodeId(3),
        relation: "1 imports",
        weight: 1,
        confidence: "aggregated"
      }
    ]);
    expect(edgeCategories.get("community-edge:1:2")).toBe("execution");
    expect(edgeCategories.get("community-edge:2:3")).toBe("dependency");
    expect(overview.nodes[0]?.degree).toBe(2);
    expect(overview.nodes[1]?.degree).toBe(1);
  });

  it("keeps the three largest relations and states how many are hidden", () => {
    const model = fixture();
    const mix = communityOverviewModel({
      ...model,
      edges: [
        { id: "mix-1", source: "auth-hub", target: "db-hub", relation: "calls" },
        { id: "mix-2", source: "auth-check", target: "db-hub", relation: "calls" },
        { id: "mix-3", source: "auth-token", target: "db-hub", relation: "calls" },
        { id: "mix-4", source: "auth-hub", target: "db-query", relation: "imports" },
        { id: "mix-5", source: "auth-check", target: "db-query", relation: "imports" },
        { id: "mix-6", source: "auth-token", target: "db-hub", relation: "contains" },
        { id: "mix-7", source: "auth-hub", target: "db-hub", relation: "references" },
        { id: "mix-8", source: "auth-hub", target: "db-query", relation: "routes" }
      ]
    }).model;
    const pair = mix.edges.find((edge) => edge.id === "community-edge:1:2");
    // Counts break first, then the palette's category order decides a tie
    // (dependency before structure before flow), and the rest is disclosed.
    expect(pair?.relation).toBe("3 calls · 2 imports · 1 references · 2 more");
    expect(pair?.weight).toBe(8);
  });

  it("ranks labels by coupling and boundary content, not size alone", () => {
    const model = fixture();
    const { importance } = communityOverviewModel({
      ...model,
      nodes: model.nodes.map((entry) =>
        entry.id === "db-hub" ? { ...entry, kind: "endpoint" } : entry)
    });
    expect(importance.get(communityNodeId(2)) ?? 0)
      .toBeGreaterThan(importance.get(communityNodeId(1)) ?? 0);
  });

  it("is deterministic for reversed input order", () => {
    const model = fixture();
    const reversed = communityOverviewModel({
      ...model,
      nodes: [...model.nodes].reverse(),
      edges: [...model.edges].reverse()
    });
    expect(communityOverviewModel(model)).toEqual(reversed);
  });

  it("sizes bubbles by member share", () => {
    expect(communityNodeSize(400, 400)).toBeGreaterThan(communityNodeSize(100, 400));
    expect(communityNodeSize(1, 400)).toBeGreaterThan(0);
  });
});

describe("community overview labels", () => {
  it("keeps the largest communities labelled on a crowded map", () => {
    const nodes = Array.from({ length: 400 }, (_, index) => node(`c-${index}`, index, 0, {
      memberCount: 400 - index
    }));
    const labelled = communityOverviewLabelledIds(nodes);
    expect(labelled.size).toBe(communityOverviewLabelLimit(nodes.length));
    expect(labelled.has("c-0")).toBe(true);
    expect(labelled.has("c-399")).toBe(false);
  });

  it("labels every community when the map is small", () => {
    const nodes = [node("a", 1, 0), node("b", 2, 0)];
    expect([...communityOverviewLabelledIds(nodes)].sort()).toEqual(["a", "b"]);
  });

  it("bounds long canvas labels", () => {
    expect(communityOverviewLabelText("Flask (flask/app.py:L110)"))
      .toBe("Flask (flask/app.…");
    expect(communityOverviewLabelText("  short \n name ")).toBe("short name");
  });
});

describe("communityDetailModel", () => {
  it("keeps only the community's members and internal relationships", () => {
    const detail = communityDetailModel(fixture(), 1);
    expect(detail?.model.nodes.map((entry) => entry.id))
      .toEqual(["auth-hub", "auth-check", "auth-token"]);
    expect(detail?.model.edges.map((edge) => edge.id)).toEqual(["e1", "e2"]);
    expect(detail?.model.stats).toEqual({
      nodes: 3,
      edges: 2,
      communities: 1,
      aggregated: false
    });
    expect(detail?.model.communities).toEqual([
      { id: 1, label: "Auth", color: "#4E79A7", hidden: false }
    ]);
    expect(detail?.bounded).toBeUndefined();
  });

  it("bounds a large community to its most connected symbols", () => {
    const model = fixture();
    const members = Array.from({ length: 40 }, (_, index) =>
      node(`member-${index}`, 7, 40 - index));
    const detail = communityDetailModel({ ...model, nodes: members }, 7, 10);
    expect(detail?.model.nodes).toHaveLength(10);
    expect(detail?.model.nodes[0]?.id).toBe("member-0");
    expect(detail?.bounded).toEqual({
      limit: 10,
      parentMembers: 40,
      currentMembers: 10
    });
    expect(COMMUNITY_DETAIL_NODE_BUDGET).toBeGreaterThan(10);
  });

  it("returns nothing for a community outside the model", () => {
    expect(communityDetailModel(fixture(), 99)).toBeUndefined();
  });
});
