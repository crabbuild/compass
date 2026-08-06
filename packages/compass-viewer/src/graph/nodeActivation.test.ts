import { describe, expect, it } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { graphNodeActivation } from "./nodeActivation";

const model: GraphViewModel = {
  schema: "compass.viewer.graph/1",
  title: "Fixture",
  stats: { nodes: 1, edges: 0, communities: 1, aggregated: true },
  nodes: [{
    id: "community-7",
    label: "Core",
    community: 7,
    memberCount: 2,
    source: { file: "src/lib.rs", startLine: 3, endLine: 8 }
  }],
  edges: [],
  communities: [{ id: 7, label: "Core", color: "#4e79a7", hidden: false }],
  hyperedges: []
};

describe("graphNodeActivation", () => {
  it("enters an aggregated community before considering source metadata", () => {
    expect(graphNodeActivation(model, model.nodes[0]!)).toEqual({
      type: "community",
      communityId: 7
    });
  });

  it("opens exact source when the same node is in a detail graph", () => {
    expect(graphNodeActivation(model, model.nodes[0]!, 7)).toEqual({
      type: "source",
      source: { file: "src/lib.rs", startLine: 3, endLine: 8 }
    });
  });

  it("does not offer a drilldown omitted from a bounded standalone export", () => {
    expect(graphNodeActivation(model, {
      ...model.nodes[0]!,
      detailAvailable: false,
      source: undefined
    })).toEqual({ type: "none" });
  });
});
