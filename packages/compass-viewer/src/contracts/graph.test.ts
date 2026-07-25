import { describe, expect, it } from "vitest";
import { GraphViewModelSchema } from "./graph";

describe("GraphViewModelSchema", () => {
  it("accepts a minimal v1 model and rejects another major schema", () => {
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

  it("preserves optional graph presentation metadata", () => {
    const parsed = GraphViewModelSchema.parse({
      schema: "compass.viewer.graph/1",
      title: "Fixture",
      stats: { nodes: 1, edges: 0, communities: 1, aggregated: true },
      nodes: [{
        id: "n1",
        label: "run",
        kind: "function",
        community: 0,
        language: "rust",
        signature: "fn run(value: usize)",
        size: 28.5,
        memberCount: 7,
        learningStatus: "preferred",
        learningStale: false,
        source: { file: "src/main.rs", startLine: 4, endLine: 8 }
      }],
      edges: [],
      communities: [{ id: 0, label: "Core", color: "#4f8cff" }]
    });
    expect(parsed.nodes[0]).toMatchObject({
      language: "rust",
      signature: "fn run(value: usize)",
      size: 28.5,
      memberCount: 7,
      learningStatus: "preferred",
      learningStale: false
    });
  });
});
