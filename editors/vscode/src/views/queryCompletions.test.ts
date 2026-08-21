import { describe, expect, it } from "vitest";
import type { CodeQueryResponse } from "@compass/viewer";
import {
  callGraphCompletionItems,
  graphCompletionItems,
  validGraphCompletionTerm
} from "./queryCompletions";

function response(): CodeQueryResponse {
  const nodes = Array.from({ length: 10 }, (_, index) => ({
    id: `node-${index}`,
    kind: index === 9 ? "type_alias" as const : "function" as const,
    roles: [],
    name: `Node${index}`,
    qualifiedName: `crate::Node${index}`,
    language: "rust",
    framework: null,
    source: {
      file: `src/node_${index}.rs`,
      startByte: 0,
      endByte: 1,
      startLine: index + 1,
      startColumn: 0,
      endLine: index + 1,
      endColumn: 1
    },
    details: null,
    evidence: []
  }));
  return {
    schema: "compass.query/1",
    operation: "search",
    results: [9, 3, 9, 0, 1, 2, 4, 5, 6, 7, 8].map((index) => ({
      nodeId: `node-${index}`,
      score: 10 - index,
      matchedFields: ["qualifiedName"]
    })),
    nodes,
    edges: [],
    files: [],
    paths: [],
    diagnostics: [],
    limits: {
      maxDepth: 1,
      maxNodes: 8,
      maxEdges: 1,
      maxPaths: 1,
      maxCandidates: 8,
      maxSourceBytes: 1,
      maxResponseBytes: 1048576
    },
    truncated: false
  };
}

describe("graphCompletionItems", () => {
  it("keeps ranked graph order, removes duplicates, and caps the response", () => {
    const items = graphCompletionItems(response());
    expect(items).toHaveLength(8);
    expect(items[0]).toEqual({
      nodeId: "node-9",
      label: "crate::Node9",
      insertText: "crate::Node9",
      detail: "type alias · src/node_9.rs:10"
    });
    expect(items.filter((item) => item.nodeId === "node-9")).toHaveLength(1);
  });

  it("drops an oversized graph identity instead of inserting a partial symbol", () => {
    const value = response();
    value.nodes[9]!.qualifiedName = "x".repeat(513);
    expect(graphCompletionItems(value).some((item) => item.nodeId === "node-9"))
      .toBe(false);
  });

  it("offers only call-capable nodes to the call graph", () => {
    const value = response();
    value.nodes[9]!.kind = "class";
    value.nodes[3]!.kind = "method";
    value.nodes[0]!.kind = "property";

    const items = callGraphCompletionItems(value);

    expect(items.map((item) => item.nodeId)).not.toContain("node-9");
    expect(items.map((item) => item.nodeId)).toEqual(expect.arrayContaining([
      "node-3",
      "node-0"
    ]));
    expect(items.every((item) => item.detail.startsWith("function")
      || item.detail.startsWith("method")
      || item.detail.startsWith("property"))).toBe(true);
  });

  it("accepts portable Unicode graph terms and rejects option-like or oversized input", () => {
    expect(validGraphCompletionTerm("支付::处理")).toBe("支付::处理");
    expect(validGraphCompletionTerm("--graph")).toBeUndefined();
    expect(validGraphCompletionTerm("$parameter")).toBeUndefined();
    expect(validGraphCompletionTerm("two words")).toBeUndefined();
    expect(validGraphCompletionTerm("x".repeat(161))).toBeUndefined();
  });
});
