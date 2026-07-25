import { describe, expect, it } from "vitest";
import { GraphViewModelSchema } from "./graph";

describe("GraphViewModelSchema", () => {
  it("accepts additive fields and rejects another major schema", () => {
    const model = {
      schema: "compass.viewer.graph/1",
      title: "Fixture",
      stats: { nodes: 1, edges: 0, communities: 1, aggregated: false },
      nodes: [{ id: "n1", label: "run", community: 0, future: true }],
      edges: [],
      communities: [{ id: 0, label: "Core", color: "#4f8cff" }],
      future: "preserved"
    };
    expect(GraphViewModelSchema.parse(model).future).toBe("preserved");
    expect(() => GraphViewModelSchema.parse({
      ...model,
      schema: "compass.viewer.graph/2"
    })).toThrow();
  });
});
