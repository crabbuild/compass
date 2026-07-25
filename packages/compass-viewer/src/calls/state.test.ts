import { describe, expect, it } from "vitest";
import type { CallGraphResponse } from "../contracts/callGraph";
import { mergeExpansion } from "./state";

const graph: CallGraphResponse = {
  schema: "compass.program.call_graph/1",
  rootSymbol: "root",
  direction: "both",
  depth: 1,
  nodes: [{ id: "root", symbol: "root", name: "run", file: null, anchor: null, graphNodeId: null, unresolved: false }],
  edges: [],
  truncated: false,
  continuations: [],
  coverage: { resolved: 0, inferred: 0, ambiguous: 0, unresolved: 0, warning: "coverage" }
};

describe("mergeExpansion", () => {
  it("is idempotent and preserves the original root", () => {
    const once = mergeExpansion(graph, graph);
    const twice = mergeExpansion(once, graph);
    expect(twice.nodes).toHaveLength(1);
    expect(twice.rootSymbol).toBe("root");
  });
});
