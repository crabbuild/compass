import { describe, expect, it } from "vitest";
import { buildCqlArgs, buildNaturalQueryArgs } from "./queryArguments";

describe("buildNaturalQueryArgs", () => {
  it("requests typed discovery JSON for readable result rendering", () => {
    expect(buildNaturalQueryArgs({
      query: "what calls save?",
      graph: "/repo/compass-out/graph.json"
    })).toEqual([
      "query", "what calls save?", "--graph", "/repo/compass-out/graph.json",
      "--format", "json"
    ]);
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
