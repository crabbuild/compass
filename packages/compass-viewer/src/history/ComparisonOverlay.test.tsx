import { describe, expect, it } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { compareGraphs } from "./ComparisonOverlay";
import { compareRecord, displayFieldValue } from "./recordDiff";

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

  it("retains exact before and after fields without presentation metadata", () => {
    const parent = graph(
      [{
        id: "shared",
        label: "Shared",
        community: 0,
        signature: "old()",
        source: { file: "src/core.ts", startLine: 2 },
        color: { background: "#111111", border: "#222222" }
      }],
      [{
        id: "edge",
        source: "shared",
        target: "shared",
        relation: "calls",
        confidence: "inferred"
      }]
    );
    const current = graph(
      [{
        id: "shared",
        label: "Shared",
        community: 0,
        signature: "new()",
        source: { file: "src/core.ts", startLine: 4 },
        color: { background: "#eeeeee", border: "#dddddd" }
      }],
      [{
        id: "edge",
        source: "shared",
        target: "shared",
        relation: "invokes",
        confidence: "extracted"
      }]
    );

    const comparison = compareGraphs(parent, current);
    const changedNode = comparison.graph.nodes[0];
    const changedEdge = comparison.graph.edges[0];

    expect(changedNode?.evidence?.fields).toEqual([
      { field: "signature", before: "old()", after: "new()" },
      { field: "source.startLine", before: 2, after: 4 }
    ]);
    expect(changedNode?.evidence?.fields.map((field) => field.field))
      .not.toContain("color.background");
    expect(changedEdge?.evidence?.fields).toEqual([
      { field: "confidence", before: "inferred", after: "extracted" },
      { field: "relation", before: "calls", after: "invokes" }
    ]);
  });

  it("preserves aggregate comparison mode for community drill-down", () => {
    const parent = graph(
      [{ id: "community-0", label: "Core", community: 0, memberCount: 5 }],
      []
    );
    const current = graph(
      [{ id: "community-0", label: "Core", community: 0, memberCount: 6 }],
      []
    );
    parent.stats.aggregated = true;
    current.stats.aggregated = true;

    const comparison = compareGraphs(parent, current);

    expect(comparison.graph.stats.aggregated).toBe(true);
    expect(comparison.graph.nodes[0]?.evidence).toMatchObject({
      before: { memberCount: 5 },
      after: { memberCount: 6 },
      fields: [{ field: "memberCount", before: 5, after: 6 }]
    });
  });

  it("matches production edges when inserted relationships shift generated IDs", () => {
    const nodes = [
      { id: "a", label: "A", community: 0 },
      { id: "b", label: "B", community: 0 },
      { id: "c", label: "C", community: 0 },
      { id: "d", label: "D", community: 0 }
    ];
    const parent = graph(nodes, [
      { id: "edge-0-b-c", source: "b", target: "c", relation: "calls" },
      { id: "edge-1-c-d", source: "c", target: "d", relation: "uses" }
    ]);
    const current = graph(nodes, [
      { id: "edge-0-a-b", source: "a", target: "b", relation: "calls" },
      { id: "edge-1-b-c", source: "b", target: "c", relation: "calls" },
      { id: "edge-2-c-d", source: "c", target: "d", relation: "uses" }
    ]);

    const comparison = compareGraphs(parent, current);

    expect(comparison).toMatchObject({
      addedEdges: 1,
      removedEdges: 0,
      changedEdges: 0
    });
    expect(comparison.graph.edges).toMatchObject([
      { id: "edge-0-a-b", source: "a", target: "b", change: "added" }
    ]);
  });

  it("retains field evidence when a shifted generated edge changes", () => {
    const nodes = [
      { id: "a", label: "A", community: 0 },
      { id: "b", label: "B", community: 0 },
      { id: "c", label: "C", community: 0 }
    ];
    const parent = graph(nodes, [
      {
        id: "edge-0-b-c",
        source: "b",
        target: "c",
        relation: "calls",
        confidence: "inferred"
      }
    ]);
    const current = graph(nodes, [
      { id: "edge-0-a-b", source: "a", target: "b", relation: "calls" },
      {
        id: "edge-1-b-c",
        source: "b",
        target: "c",
        relation: "uses",
        confidence: "extracted"
      }
    ]);

    const comparison = compareGraphs(parent, current);

    expect(comparison).toMatchObject({
      addedEdges: 1,
      removedEdges: 0,
      changedEdges: 1
    });
    expect(comparison.graph.edges.find((edge) => edge.change === "changed")?.evidence?.fields)
      .toEqual([
        { field: "confidence", before: "inferred", after: "extracted" },
        { field: "relation", before: "calls", after: "uses" }
      ]);
  });
});

describe("record diff presentation", () => {
  it("is independent of object key order and reports missing nested values", () => {
    const evidence = compareRecord(
      { source: { startLine: 2, file: "src/core.ts" }, signature: "same()" },
      { signature: "same()", source: { file: "src/core.ts", endLine: 8 } }
    );

    expect(evidence.fields).toEqual([
      { field: "source.endLine", after: 8 },
      { field: "source.startLine", before: 2 }
    ]);
  });

  it("bounds structured values and marks them as shortened", () => {
    expect(displayFieldValue({ detail: "x".repeat(100) }, 32)).toEqual({
      text: expect.stringMatching(/…$/),
      truncated: true
    });
  });
});
