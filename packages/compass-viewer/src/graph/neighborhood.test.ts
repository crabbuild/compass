import { describe, expect, it } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import {
  clampNeighborhoodDepth,
  graphNeighborhood
} from "./neighborhood";

const model: GraphViewModel = {
  schema: "compass.viewer.graph/1",
  title: "Traversal fixture",
  stats: { nodes: 5, edges: 5, communities: 1, aggregated: false },
  nodes: ["root", "child", "leaf", "caller", "side"].map((id) => ({
    id,
    label: id,
    community: 0
  })),
  edges: [
    { id: "root-child", source: "root", target: "child", relation: "calls" },
    { id: "child-leaf", source: "child", target: "leaf", relation: "calls" },
    { id: "caller-root", source: "caller", target: "root", relation: "calls" },
    { id: "root-side", source: "root", target: "side", relation: "imports" },
    { id: "side-root", source: "side", target: "root", relation: "tests" }
  ],
  communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
  hyperedges: []
};

describe("graphNeighborhood", () => {
  it("preserves outgoing direction and exact depth", () => {
    const neighborhood = graphNeighborhood(model, "root", 2, "outgoing");
    expect([...neighborhood.nodeIds].sort()).toEqual(["child", "leaf", "root", "side"]);
    expect([...neighborhood.edgeIds].sort()).toEqual([
      "child-leaf",
      "root-child",
      "root-side",
      "side-root"
    ]);
  });

  it("preserves incoming direction", () => {
    const neighborhood = graphNeighborhood(model, "root", 1, "incoming");
    expect([...neighborhood.nodeIds].sort()).toEqual(["caller", "root", "side"]);
    expect([...neighborhood.edgeIds].sort()).toEqual(["caller-root", "side-root"]);
  });

  it("traverses both directions without inventing edges", () => {
    const neighborhood = graphNeighborhood(model, "root", 1, "both");
    expect([...neighborhood.nodeIds].sort()).toEqual([
      "caller",
      "child",
      "root",
      "side"
    ]);
    expect([...neighborhood.edgeIds].sort()).toEqual([
      "caller-root",
      "root-child",
      "root-side",
      "side-root"
    ]);
  });

  it("clamps traversal to the bounded 1–4 hop range", () => {
    expect(clampNeighborhoodDepth(-2)).toBe(1);
    expect(clampNeighborhoodDepth(3.9)).toBe(3);
    expect(clampNeighborhoodDepth(99)).toBe(4);
  });
});
