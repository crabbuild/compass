import { describe, expect, it } from "vitest";
import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";
import { communityOverviewModel } from "./communityOverview";
import {
  communityMatrixCells,
  communityMatrixSelection,
  communityVariantData,
  layoutCommunityLanes,
  layoutCommunityTreemap,
  laneRibbons
} from "./communityVariants";

function node(id: string, community: number, degree: number, extra: Partial<GraphNode> = {}): GraphNode {
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

function model(nodes: GraphNode[], edges: GraphEdge[]): GraphViewModel {
  const communityIds = [...new Set(nodes.map((entry) => entry.community))];
  return {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes: nodes.length, edges: edges.length, communities: communityIds.length, aggregated: false },
    nodes,
    edges,
    communities: communityIds.map((id) => ({
      id,
      label: `Community ${id}`,
      color: `#${(id + 1).toString(16).repeat(6).slice(0, 6)}`,
      hidden: false
    })),
    hyperedges: []
  };
}

function fixture(): GraphViewModel {
  const nodes: GraphNode[] = [
    node("auth-hub", 1, 12, { communityName: "Auth" }),
    node("auth-check", 1, 4),
    node("auth-token", 1, 2),
    node("db-hub", 2, 9, { communityName: "Database", kind: "database_table" }),
    node("db-query", 2, 3),
    node("ui-hub", 3, 5, { communityName: "Interface" })
  ];
  const edges: GraphEdge[] = [
    { id: "e1", source: "auth-hub", target: "auth-check", relation: "calls" },
    { id: "e2", source: "auth-hub", target: "db-hub", relation: "calls" },
    { id: "e3", source: "auth-check", target: "db-hub", relation: "imports" },
    { id: "e4", source: "db-query", target: "ui-hub", relation: "contains" }
  ];
  return model(nodes, edges);
}

function overviewFixture() {
  const source = fixture();
  return { source, overview: communityOverviewModel(source) };
}

describe("communityVariantData", () => {
  it("returns nothing for a graph with no overview to describe", () => {
    expect(communityVariantData(fixture())).toBeUndefined();
  });

  it("projects a derived overview in importance order with typed links", () => {
    const { overview } = overviewFixture();
    const data = communityVariantData(overview.model, overview);
    if (!data) throw new Error("variant data missing");

    expect(data.communities.map((community) => community.id)).toEqual([2, 1, 3]);
    expect(data.communities.map((community) => community.memberCount)).toEqual([2, 3, 1]);
    expect(data.communities[0]?.label).toBe("Database");
    expect(data.symbols).toBe(6);
    // One link per community pair: the mix stays in the relation text and the
    // dominant kind (execution, before dependency on a tie) drives the colour.
    expect(data.links.map((link) => [link.source, link.target, link.category])).toEqual([
      [1, 2, "execution"],
      [2, 3, "structure"]
    ]);
    expect(data.links[0]?.relation).toBe("1 calls · 1 imports");
    expect(data.links[0]?.weight).toBe(2);
    expect(data.byId.get(2)?.memberCount).toBe(2);
  });

  it("describes an already aggregated export without inventing kinds", () => {
    const aggregated: GraphViewModel = {
      ...model([], []),
      stats: { nodes: 2, edges: 1, communities: 2, aggregated: true },
      nodes: [
        node("1", 1, 1, { kind: "community", memberCount: 40, communityName: "Core" }),
        node("2", 2, 1, { kind: "community", memberCount: 10, communityName: "Tests" })
      ],
      edges: [{
        id: "aggregate-1",
        source: "1",
        target: "2",
        relation: "3 cross-community edges",
        weight: 3,
        confidence: "aggregated"
      }]
    };
    const data = communityVariantData(aggregated);
    if (!data) throw new Error("variant data missing");

    expect(data.communities.map((community) => [community.label, community.memberCount]))
      .toEqual([["Core", 40], ["Tests", 10]]);
    expect(data.symbols).toBe(50);
    expect(data.links).toEqual([{
      id: "aggregate-1",
      source: 1,
      target: 2,
      weight: 3,
      relation: "3 cross-community edges",
      category: "other"
    }]);
  });
});

describe("community matrix", () => {
  it("bounds the shown communities and reports the rest", () => {
    const { overview } = overviewFixture();
    const data = communityVariantData(overview.model, overview)!;
    const selection = communityMatrixSelection(data, 2);
    expect(selection.shown.map((community) => community.id)).toEqual([2, 1]);
    expect(selection.omitted).toBe(1);
  });

  it("builds a symmetric grid that resolves the same link on both axes", () => {
    const { overview } = overviewFixture();
    const data = communityVariantData(overview.model, overview)!;
    const cells = communityMatrixCells(data, 3);
    expect(cells).toHaveLength(9);
    const forward = cells.find((cell) => cell.row.id === 1 && cell.column.id === 2);
    const backward = cells.find((cell) => cell.row.id === 2 && cell.column.id === 1);
    expect(forward?.link?.id).toBe(backward?.link?.id);
    const self = cells.find((cell) => cell.row.id === 2 && cell.column.id === 2);
    expect(self?.link).toBeUndefined();
  });
});

describe("community treemap", () => {
  it("keeps tile area proportional to symbols", () => {
    const { overview } = overviewFixture();
    const data = communityVariantData(overview.model, overview)!;
    const { tiles } = layoutCommunityTreemap(data, 10, { width: 800, height: 400 });
    const area = tiles.reduce((sum, tile) => sum + tile.width * tile.height, 0);
    expect(area).toBeCloseTo(800 * 400, 3);
    for (const tile of tiles) {
      const expected = (tile.community.memberCount / data.symbols) * 800 * 400;
      expect(tile.width * tile.height).toBeCloseTo(expected, 3);
    }
  });

  it("discloses the communities it folds into one tail tile", () => {
    const { overview } = overviewFixture();
    const data = communityVariantData(overview.model, overview)!;
    const { tiles, omitted } = layoutCommunityTreemap(data, 2, { width: 800, height: 400 });
    expect(tiles).toHaveLength(3);
    const tail = tiles.at(-1)!;
    expect(tail.community.id).toBe(-1);
    expect(tail.community.label).toContain("more");
    expect(tail.community.memberCount).toBe(omitted.symbols);
    expect(omitted).toEqual({ communities: 1, symbols: 1 });
  });

  it("is deterministic", () => {
    const { overview } = overviewFixture();
    const data = communityVariantData(overview.model, overview)!;
    const first = layoutCommunityTreemap(data, 10, { width: 800, height: 400 });
    const second = layoutCommunityTreemap(data, 10, { width: 800, height: 400 });
    expect(first.tiles.map((tile) => [tile.community.id, tile.x, tile.y, tile.width, tile.height]))
      .toEqual(second.tiles.map((tile) => [tile.community.id, tile.x, tile.y, tile.width, tile.height]));
  });
});

describe("community lanes", () => {
  it("leads with the most important tier and discloses the tail", () => {
    const { overview } = overviewFixture();
    const data = communityVariantData(overview.model, overview)!;
    const layout = layoutCommunityLanes(data, 2, 1);
    expect(layout.lanes[0]?.entries[0]?.community.id).toBe(2);
    expect(layout.lanes[1]?.entries[0]?.community.id).toBe(1);
    expect(layout.omitted).toEqual({ communities: 1, symbols: 1 });
    const tail = layout.lanes.at(-1)?.entries[0];
    expect(tail?.community.id).toBe(-1);
    expect(tail?.width).toBe(1);
  });

  it("bounds and orders the coupling ribbons", () => {
    const { overview } = overviewFixture();
    const data = communityVariantData(overview.model, overview)!;
    const layout = layoutCommunityLanes(data);
    const ribbons = laneRibbons(data, layout, 2);
    expect(ribbons).toHaveLength(2);
    expect(ribbons.map((ribbon) => ribbon.link.id))
      .toEqual(["community-edge:1:2", "community-edge:2:3"]);
    expect(laneRibbons(data, layout, 2)).toEqual(ribbons);
  });
});
