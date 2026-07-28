import { describe, expect, it } from "vitest";
import {
  callGraphCommandArguments,
  callGraphExpansionArguments,
  callGraphRootArguments
} from "./callGraphArguments";

describe("Compass call graph arguments", () => {
  it("passes the cursor file, UTF-8 byte, line, and direction as separate arguments", () => {
    expect(callGraphRootArguments({
      file: "cmd/entire/cli/auth/control plane.go",
      byte: 1683,
      line: 42
    }, "callees", 2)).toEqual([
      "--file",
      "cmd/entire/cli/auth/control plane.go",
      "--byte",
      "1683",
      "--line",
      "42",
      "--direction",
      "callees",
      "--depth",
      "2"
    ]);
  });

  it("expands a graph from its stable symbol identifier", () => {
    expect(callGraphExpansionArguments("go:auth.Resolve", "callers", 3)).toEqual([
      "--symbol",
      "go:auth.Resolve",
      "--direction",
      "callers",
      "--depth",
      "3"
    ]);
  });

  it("keeps interactive resolution on the structural graph without loading Program IR", () => {
    expect(callGraphCommandArguments(
      ["--symbol", "go:auth.Resolve", "--direction", "both", "--depth", "2"],
      "/repo/compass-out/graph.json"
    )).toEqual([
      "call-graph",
      "--symbol",
      "go:auth.Resolve",
      "--direction",
      "both",
      "--depth",
      "2",
      "--max-nodes",
      "500",
      "--max-edges",
      "1000",
      "--graph",
      "/repo/compass-out/graph.json",
      "--format",
      "json"
    ]);
  });
});
