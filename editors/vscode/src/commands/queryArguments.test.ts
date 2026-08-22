import { describe, expect, it } from "vitest";
import {
  buildAskArgs,
  buildCompletionArgs,
  buildCqlArgs,
  buildExplainArgs
} from "./queryArguments";

describe("buildAskArgs", () => {
  it("uses the typed ask contract for the working tree", () => {
    expect(buildAskArgs({
      query: "who calls checkout?",
      graph: "/repo/compass-out/graph.json"
    })).toEqual([
      "ask", "who calls checkout?",
      "--graph", "/repo/compass-out/graph.json",
      "--format", "json"
    ]);
  });

  it("selects an immutable revision without mixing graph sources", () => {
    expect(buildAskArgs({ query: "who calls checkout?", revision: "HEAD~2" }))
      .toEqual([
        "ask", "who calls checkout?", "--at", "HEAD~2", "--format", "json"
      ]);
  });
});

describe("buildExplainArgs", () => {
  it("requests a readable explanation for one exact symbol", () => {
    expect(buildExplainArgs({
      query: "crate::Checkout::run",
      graph: "/repo/compass-out/graph.json"
    })).toEqual([
      "explain", "crate::Checkout::run",
      "--graph", "/repo/compass-out/graph.json"
    ]);
  });
});

describe("buildCompletionArgs", () => {
  it("runs a tightly bounded typed search against the working graph", () => {
    expect(buildCompletionArgs({
      term: "Payment",
      graph: "/repo/compass-out/graph.json"
    })).toEqual([
      "search", "Payment",
      "--max-depth", "1",
      "--max-nodes", "8",
      "--max-edges", "1",
      "--max-paths", "1",
      "--max-candidates", "8",
      "--max-source-bytes", "1",
      "--max-response-bytes", "1048576",
      "--graph", "/repo/compass-out/graph.json",
      "--format", "json"
    ]);
  });

  it("searches the selected immutable revision without mixing graph sources", () => {
    expect(buildCompletionArgs({ term: "Payment", revision: "HEAD~2" }))
      .toEqual(expect.arrayContaining([
        "search", "Payment", "--max-depth", "1", "--at", "HEAD~2", "--format", "json"
      ]));
  });
});

describe("buildCqlArgs", () => {
  it("keeps the query and parameters as literal arguments", () => {
    expect(buildCqlArgs({
      query: "MATCH (n) RETURN n LIMIT 5",
      params: { kind: "Function" },
      timeoutMs: 5000,
      maxRows: 100
    })).toEqual([
      "query", "--cql", "MATCH (n) RETURN n LIMIT 5",
      "--param", "kind=Function", "--timeout-ms", "5000",
      "--max-rows", "100", "--format", "json"
    ]);
  });
});
