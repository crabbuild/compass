import { describe, expect, it } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { compareGraphs } from "./ComparisonOverlay";

function graph(
  nodes: GraphViewModel["nodes"],
  edges: GraphViewModel["edges"]
): GraphViewModel {
  return {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: {
      nodes: nodes.length,
      edges: edges.length,
      communities: 1,
      aggregated: false
    },
    nodes,
    edges,
    communities: [{ id: 0, label: "Core", color: "#6688aa", hidden: false }],
    hyperedges: []
  };
}

describe("compareGraphs", () => {
  it("builds a focused, color-coded delta graph with changed records and their endpoints", () => {
    const parent = graph(
      [
        { id: "shared", label: "Shared", community: 0, signature: "old" },
        { id: "removed", label: "Removed", community: 0 }
      ],
      [{
        id: "shared-edge",
        source: "shared",
        target: "removed",
        relation: "calls",
        confidence: "inferred"
      }]
    );
    const current = graph(
      [
        { id: "shared", label: "Shared", community: 0, signature: "new" },
        { id: "added", label: "Added", community: 0 }
      ],
      [{
        id: "shared-edge",
        source: "shared",
        target: "added",
        relation: "calls",
        confidence: "extracted"
      }]
    );

    const comparison = compareGraphs(parent, current);

    expect(comparison).toMatchObject({
      addedNodes: 1,
      removedNodes: 1,
      changedNodes: 1,
      addedEdges: 0,
      removedEdges: 0,
      changedEdges: 1
    });
    expect(comparison.graph.nodes.map((node) => [node.id, node.change])).toEqual([
      ["added", "added"],
      ["removed", "removed"],
      ["shared", "changed"]
    ]);
    expect(comparison.graph.edges.map((edge) => [edge.id, edge.change])).toEqual([
      ["shared-edge", "changed"]
    ]);
    expect(comparison.graph.stats).toMatchObject({ nodes: 3, edges: 1 });
  });

  it("returns an empty delta graph when two visible graphs are structurally identical", () => {
    const unchanged = graph(
      [{ id: "same", label: "Same", community: 0 }],
      []
    );

    const comparison = compareGraphs(unchanged, unchanged);

    expect(comparison.graph.nodes).toEqual([]);
    expect(comparison.graph.edges).toEqual([]);
    expect(comparison.graph.stats).toMatchObject({ nodes: 0, edges: 0 });
  });
});
