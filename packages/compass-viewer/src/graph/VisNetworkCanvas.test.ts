import { describe, expect, it } from "vitest";
import type { GraphNode, GraphViewModel } from "../contracts/graph";
import {
  formatGraphEdgeLabel,
  shouldShowGraphEdgeLabel
} from "./edgeLabels";
import { graphNodeColor } from "./VisNetworkCanvas";

const model: GraphViewModel = {
  schema: "compass.viewer.graph/1",
  title: "Fixture",
  stats: { nodes: 1, edges: 0, communities: 1, aggregated: false },
  nodes: [],
  edges: [],
  communities: [{ id: 2, label: "Core", color: "#4E79A7", hidden: false }],
  hyperedges: []
};
const node: GraphNode = {
  id: "run",
  label: "run",
  community: 2,
  color: { background: "#112233", border: "#445566" }
};

describe("graphNodeColor", () => {
  it("preserves the Compass export node palette by default", () => {
    expect(graphNodeColor(model, node)).toEqual({
      background: "#112233",
      border: "#445566"
    });
  });

  it("uses the VS Code contrast border without replacing the node fill", () => {
    expect(graphNodeColor(model, node, "#ffffff")).toEqual({
      background: "#112233",
      border: "#ffffff"
    });
  });

  it("uses separate soft fills and strong borders for comparison status colors", () => {
    expect(graphNodeColor(model, { ...node, change: "changed" }, undefined, {
      added: { background: "#dafbe1", border: "#1a7f37" },
      removed: { background: "#ffebe9", border: "#cf222e" },
      changed: { background: "#f7edcf", border: "#9a6700" },
      unchanged: { background: "#e7e9eb", border: "#656d76" }
    })).toEqual({
      background: "#f7edcf",
      border: "#9a6700"
    });
  });

  it("derives focused relationship labels without forcing unrelated edges", () => {
    const edge = {
      id: "run-helper",
      source: "run",
      target: "helper",
      relation: "calls",
      confidence: "extracted" as const
    };
    expect(formatGraphEdgeLabel(edge)).toBe("calls [EXTRACTED]");
    expect(shouldShowGraphEdgeLabel(edge, {
      forceLabels: false,
      focusedNodeId: "run",
      focusedEdgeId: null,
      hoveredEdgeId: null,
    })).toBe(true);
    expect(shouldShowGraphEdgeLabel(edge, {
      forceLabels: false,
      focusedNodeId: "store",
      focusedEdgeId: null,
      hoveredEdgeId: null,
    })).toBe(false);
  });
});
