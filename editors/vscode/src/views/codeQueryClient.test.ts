import { describe, expect, it, vi } from "vitest";
import {
  codeQueryArguments,
  codeQueryRequiresRebuild,
  runCodeQuery
} from "./codeQueryClient";

describe("codeQueryClient", () => {
  it("passes symbols and paths as literal arguments with bounded JSON output", () => {
    expect(codeQueryArguments(
      { operation: "explore", symbols: ["User Service", "handler; rm -rf nowhere"] },
      "/repo/compass-out/graph.json",
      "/repo with spaces"
    )).toEqual([
      "explore",
      "User Service",
      "handler; rm -rf nowhere",
      "--root",
      "/repo with spaces",
      "--graph",
      "/repo/compass-out/graph.json",
      "--max-depth",
      "8",
      "--max-nodes",
      "500",
      "--max-edges",
      "1000",
      "--max-paths",
      "100",
      "--max-source-bytes",
      "1048576",
      "--max-response-bytes",
      "8388608",
      "--format",
      "json"
    ]);
  });

  it("rejects values that could be confused with Compass options", () => {
    expect(() => codeQueryArguments(
      { operation: "search", query: "--graph" },
      "/repo/graph.json",
      "/repo"
    )).toThrow("cannot begin with '--'");
  });

  it("validates responses and forwards cancellation to the process manager", async () => {
    const signal = new AbortController().signal;
    const runJson = vi.fn().mockResolvedValue({
      schema: "compass.query/1",
      operation: "search",
      results: [],
      nodes: [],
      edges: [],
      files: [],
      paths: [],
      diagnostics: [],
      limits: {
        maxDepth: 8,
        maxNodes: 500,
        maxEdges: 1000,
        maxPaths: 100,
        maxCandidates: 20,
        maxSourceBytes: 1048576,
        maxResponseBytes: 8388608
      },
      truncated: false
    });
    const session = {
      root: "/repo",
      graphPath: "/repo/compass-out/graph.json",
      processes: { runJson }
    };
    await runCodeQuery(session as never, { operation: "search", query: "checkout" }, signal);
    expect(runJson.mock.calls[0]?.[1]).toEqual(expect.arrayContaining([
      "search", "checkout", "--format", "json"
    ]));
    expect(runJson.mock.calls[0]?.[3]).toBe(signal);
  });

  it("recognizes hard-cutover diagnostics without treating ordinary empties as errors", () => {
    expect(codeQueryRequiresRebuild(
      "unsupported graph schema; Compass requires compass.graph/1. Run compass update to rebuild"
    )).toBe(true);
    expect(codeQueryRequiresRebuild("No symbol matched checkout")).toBe(false);
  });
});
