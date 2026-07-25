import { describe, expect, it } from "vitest";
import type { GraphNode, GraphViewModel } from "../contracts/graph";
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
});
