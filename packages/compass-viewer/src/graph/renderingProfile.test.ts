import { describe, expect, it } from "vitest";
import type { GraphNode, GraphViewModel } from "../contracts/graph";
import {
  graphRenderingProfile,
  seedStaticGraphPositions,
  STATIC_LAYOUT_EDGE_THRESHOLD,
  STATIC_LAYOUT_NODE_THRESHOLD
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
});
