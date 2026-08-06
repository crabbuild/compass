import { describe, expect, it } from "vitest";
import type { GraphNode, GraphViewModel } from "../contracts/graph";
import {
  AGGREGATED_EDGE_RENDER_LIMIT,
  graphRenderingProfile,
  seedGraphLayoutPositions,
  seedStaticGraphPositions,
  STATIC_LAYOUT_EDGE_THRESHOLD,
  STATIC_LAYOUT_NODE_THRESHOLD,
  visibleGraphEdges
} from "./renderingProfile";

function model(nodes: number, edges: number): GraphViewModel {
  return {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes, edges, communities: 1, aggregated: false },
    nodes: Array.from({ length: nodes }, (_, index) => ({
      id: `n-${index}`,
      label: `Node ${index}`,
      community: 0
    })),
    edges: Array.from({ length: edges }, (_, index) => ({
      id: `e-${index}`,
      source: "n-0",
      target: "n-1",
      relation: "calls"
    })),
    communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
    hyperedges: []
  };
}

describe("graphRenderingProfile", () => {
  it("keeps small and sparse graphs interactive", () => {
    expect(graphRenderingProfile(model(
      STATIC_LAYOUT_NODE_THRESHOLD - 1,
      STATIC_LAYOUT_EDGE_THRESHOLD - 1
    ))).toBe("interactive");
  });

  it("selects static rendering for either a large or dense graph", () => {
    expect(graphRenderingProfile(model(STATIC_LAYOUT_NODE_THRESHOLD, 0))).toBe("static");
    expect(graphRenderingProfile(model(2, STATIC_LAYOUT_EDGE_THRESHOLD))).toBe("static");
  });
});

describe("seedStaticGraphPositions", () => {
  it("produces stable positions independent of input order", () => {
    const nodes: GraphNode[] = [
      { id: "beta", label: "Beta", community: 2 },
      { id: "alpha", label: "Alpha", community: 1 },
      { id: "gamma", label: "Gamma", community: 1 }
    ];

    expect([...seedStaticGraphPositions(nodes)]).toEqual([
      ...seedStaticGraphPositions([...nodes].reverse())
    ]);
  });

  it("assigns every node a distinct position", () => {
    const nodes = model(1_500, 0).nodes;
    const positions = seedStaticGraphPositions(nodes);
    const coordinates = new Set(
      [...positions.values()].map(({ x, y }) => `${x.toFixed(6)},${y.toFixed(6)}`)
    );

    expect(positions.size).toBe(nodes.length);
    expect(coordinates.size).toBe(nodes.length);
  });

  it("places aggregated communities in a deterministic hub-centered disc", () => {
    const nodes: GraphNode[] = Array.from({ length: 400 }, (_, index) => ({
      id: `community-${index}`,
      label: `Community ${index}`,
      community: index,
      degree: index === 237 ? 10_000 : index % 17,
      memberCount: 400 - index
    }));
    const positions = seedStaticGraphPositions(nodes, true);
    const reversed = seedStaticGraphPositions([...nodes].reverse(), true);
    expect([...positions]).toEqual([...reversed]);
    expect(positions.get("community-237")).toEqual({ x: 0, y: 0 });

    const coordinates = [...positions.values()];
    const width = Math.max(...coordinates.map(({ x }) => x))
      - Math.min(...coordinates.map(({ x }) => x));
    const height = Math.max(...coordinates.map(({ y }) => y))
      - Math.min(...coordinates.map(({ y }) => y));
    expect(Math.max(width, height) / Math.min(width, height)).toBeLessThan(1.15);
  });
});

describe("seedGraphLayoutPositions", () => {
  const nodes: GraphNode[] = [
    { id: "gamma", label: "Gamma", community: 2 },
    { id: "alpha", label: "Alpha", community: 1 },
    { id: "beta", label: "Beta", community: 1 }
  ];

  it.each(["circle", "concentric", "spiral", "grid"] as const)(
    "produces a deterministic %s layout",
    (style) => {
      expect([...seedGraphLayoutPositions(nodes, style)]).toEqual([
        ...seedGraphLayoutPositions([...nodes].reverse(), style)
      ]);
    }
  );

  it("centers the highest-degree node and expands through concentric rings", () => {
    const hubs: GraphNode[] = Array.from({ length: 24 }, (_, index) => ({
      id: `hub-${index.toString().padStart(2, "0")}`,
      label: `Hub ${index}`,
      community: index % 3,
      degree: index
    }));
    const positions = seedGraphLayoutPositions(hubs, "concentric");
    expect(positions.get("hub-23")).toEqual({ x: 0, y: 0 });
    const radii = [...positions.values()].map(({ x, y }) => Math.hypot(x, y));
    expect(new Set(radii.map((radius) => Math.round(radius)))).toEqual(
      new Set([0, 72, 144])
    );
  });

  it("places spiral nodes at monotonically increasing radii", () => {
    const positions = seedGraphLayoutPositions(nodes, "spiral");
    const radii = [...positions.values()].map(({ x, y }) => Math.hypot(x, y));
    expect(radii).toEqual([...radii].sort((left, right) => left - right));
  });

  it("places grid nodes at distinct coordinates", () => {
    const positions = seedGraphLayoutPositions(model(5_200, 0).nodes, "grid");
    const coordinates = new Set(
      [...positions.values()].map(({ x, y }) => `${x},${y}`)
    );
    expect(positions.size).toBe(5_200);
    expect(coordinates.size).toBe(5_200);
  });

  it("aligns natural community blocks in outer rows and columns", () => {
    const groupedNodes: GraphNode[] = Array.from({ length: 16 }, (_, index) => ({
      id: `grouped-${index}`,
      label: `Grouped ${index}`,
      community: Math.floor(index / 4)
    }));
    const positions = seedGraphLayoutPositions(groupedNodes, "grid");
    const first = positions.get("grouped-0")!;
    const nextColumn = positions.get("grouped-4")!;
    const nextRow = positions.get("grouped-8")!;

    expect(first.y).toBe(nextColumn.y);
    expect(first.x).toBe(nextRow.x);
    expect(nextColumn.x - first.x).toBeGreaterThan(112);
    expect(nextRow.y - first.y).toBeGreaterThan(112);
  });

  it("batches aggregated communities into aligned four-by-four blocks", () => {
    const communities: GraphNode[] = Array.from({ length: 40 }, (_, index) => ({
      id: `community-${index}`,
      label: `Community ${index}`,
      community: index
    }));
    const positions = seedGraphLayoutPositions(communities, "grid", true);
    const blockCenter = (start: number, end: number) => {
      const block = Array.from({ length: end - start }, (_, offset) =>
        positions.get(`community-${start + offset}`)!);
      return {
        x: (Math.min(...block.map(({ x }) => x)) + Math.max(...block.map(({ x }) => x))) / 2,
        y: (Math.min(...block.map(({ y }) => y)) + Math.max(...block.map(({ y }) => y))) / 2
      };
    };
    const first = blockCenter(0, 16);
    const nextColumn = blockCenter(16, 32);
    const nextRow = blockCenter(32, 40);

    expect(first.y).toBe(nextColumn.y);
    expect(first.x).toBe(nextRow.x);
    expect(nextColumn.x - first.x).toBeGreaterThan(4 * 56);
    expect(nextRow.y - first.y).toBeGreaterThan(4 * 56);
  });
});

describe("visibleGraphEdges", () => {
  it("keeps a deterministic bounded backbone for dense community overviews", () => {
    const dense = model(100, 5_000);
    dense.stats.aggregated = true;
    dense.edges = dense.edges.map((edge, index) => ({
      ...edge,
      source: `n-${index % 100}`,
      target: `n-${(index * 17 + 1) % 100}`,
      weight: index + 1,
      relation: `${index + 1} cross-community edges`
    }));
    const selected = visibleGraphEdges(dense);
    expect(selected).toHaveLength(AGGREGATED_EDGE_RENDER_LIMIT);
    expect(selected[0]?.weight).toBe(5_000);
    expect(selected.map((edge) => edge.id)).toEqual(
      visibleGraphEdges({ ...dense, edges: [...dense.edges].reverse() })
        .map((edge) => edge.id)
    );
  });

  it("preserves every edge outside an aggregated overview", () => {
    const exact = model(100, 5_000);
    expect(visibleGraphEdges(exact)).toBe(exact.edges);
  });
});
