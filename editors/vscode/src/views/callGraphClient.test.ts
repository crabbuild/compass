import { describe, expect, it, vi } from "vitest";
import {
  runCallGraph,
  runCallGraphAtCursor,
  runCallGraphForSymbol
} from "./callGraphClient";

const source = {
  file: "crates/globset/src/glob.rs",
  startByte: 4695,
  endByte: 4802,
  startLine: 147,
  startColumn: 4,
  endLine: 149,
  endColumn: 5
};
const evidence = {
  layer: "structural_graph",
  origin: "ast",
  extractor: "compass.languages.rust.universal",
  confidence: "exact",
  anchor: source,
  rule: null,
  wiringSite: null,
  resolution: "exact"
};
const limits = {
  maxDepth: 8,
  maxNodes: 500,
  maxEdges: 1000,
  maxPaths: 100,
  maxCandidates: 20,
  maxSourceBytes: 1_048_576,
  maxResponseBytes: 8_388_608
};

function queryResponse(operation: "callers" | "callees") {
  const root = {
    id: "sha256:method",
    kind: "method",
    roles: [],
    name: ".is_match_candidate()",
    qualifiedName: "globset::glob::GlobMatcher::is_match_candidate",
    language: "rust",
    framework: null,
    source,
    details: null,
    evidence: [evidence]
  };
  const related = {
    ...root,
    id: operation === "callers" ? "sha256:caller" : "sha256:callee",
    name: operation === "callers" ? ".is_match()" : "is_match",
    qualifiedName: operation === "callers"
      ? "globset::glob::GlobMatcher::is_match"
      : "globset::pathutil::is_match"
  };
  const caller = operation === "callers" ? related : root;
  const callee = operation === "callees" ? related : root;
  return {
    schema: "compass.query/1",
    operation,
    results: [],
    nodes: [root, related],
    edges: [{
      id: `${caller.id}:${callee.id}`,
      source: caller.id,
      target: callee.id,
      kind: "calls",
      relationshipSite: source,
      details: null,
      evidence: [evidence]
    }],
    files: [],
    paths: [],
    diagnostics: [],
    limits,
    truncated: false
  };
}

describe("callGraphClient", () => {
  it("returns the stable cursor root and forwards bounded graph arguments", async () => {
    const signal = new AbortController().signal;
    const runJson = vi.fn().mockResolvedValue({
      schema: "compass.call_graph/1",
      rootSymbol: "sha256:root",
      direction: "both",
      depth: 1,
      nodes: [],
      edges: [],
      truncated: false,
      continuations: [],
      coverage: {
        resolved: 0,
        inferred: 0,
        ambiguous: 0,
        unresolved: 0,
        evidenceLayer: "structural_graph",
        partial: true,
        limitations: [],
        warning: "Structural graph"
      }
    });
    const session = {
      root: "/repo",
      graphPath: "/repo/compass-out/graph.json",
      processes: { runJson }
    };

    const response = await runCallGraph(
      session as never,
      [
        "--file", "src/lib.rs", "--byte", "120", "--line", "8",
        "--direction", "both", "--depth", "1"
      ],
      signal
    );

    expect(response.rootSymbol).toBe("sha256:root");
    expect(runJson.mock.calls[0]?.[1]).toEqual([
      "call-graph",
      "--file", "src/lib.rs", "--byte", "120", "--line", "8",
      "--direction", "both", "--depth", "1",
      "--max-nodes", "500",
      "--max-edges", "1000",
      "--graph", "/repo/compass-out/graph.json",
      "--format", "json"
    ]);
    expect(runJson.mock.calls[0]?.[3]).toBe(signal);
  });

  it("opens a bounded call graph directly from a symbol name", async () => {
    const runJson = vi.fn().mockResolvedValue({
      schema: "compass.call_graph/1",
      rootSymbol: "sha256:root",
      direction: "callees",
      depth: 2,
      nodes: [],
      edges: [],
      truncated: false,
      continuations: [],
      coverage: {
        resolved: 0,
        inferred: 0,
        ambiguous: 0,
        unresolved: 0,
        evidenceLayer: "structural_graph",
        partial: false,
        limitations: [],
        warning: null
      }
    });
    const session = {
      root: "/repo",
      graphPath: "/repo/compass-out/graph.json",
      processes: { runJson }
    };

    await runCallGraphForSymbol(
      session as never,
      "globset::GlobMatcher::is_match",
      "callees",
      2
    );

    expect(runJson.mock.calls[0]?.[1]).toEqual([
      "call-graph",
      "--symbol", "globset::GlobMatcher::is_match",
      "--direction", "callees",
      "--depth", "2",
      "--max-nodes", "500",
      "--max-edges", "1000",
      "--graph", "/repo/compass-out/graph.json",
      "--format", "json"
    ]);
  });

  it("falls back to a typed cursor lookup for Compass 0.3.0 graphs", async () => {
    const runJson = vi.fn()
      .mockResolvedValueOnce({
        schema: "compass.cql.result/1",
        columns: [],
        rows: [{
          "n.id": { type: "string", value: "sha256:method" },
          "n.kind": { type: "string", value: "method" },
          "n.source": {
            type: "map",
            value: {
              file: { type: "string", value: "crates/globset/src/glob.rs" },
              startByte: { type: "integer", value: 4695 },
              endByte: { type: "integer", value: 4802 },
              startLine: { type: "integer", value: 147 },
              startColumn: { type: "integer", value: 4 },
              endLine: { type: "integer", value: 149 },
              endColumn: { type: "integer", value: 5 }
            }
          }
        }],
        profile: null,
        explain: null
      })
      .mockResolvedValueOnce(queryResponse("callers"))
      .mockResolvedValueOnce(queryResponse("callees"));
    const session = {
      root: "/repo",
      graphPath: "/repo/compass-out/graph.json",
      processes: { runJson },
      capabilities: { compass_version: "0.3.0" }
    };

    const response = await runCallGraphAtCursor(
      session as never,
      {
        file: "crates/globset/src/glob.rs",
        byte: 4707,
        line: 147
      },
      "both",
      1
    );

    expect(response.rootSymbol).toBe("sha256:method");
    expect(response.nodes.map((node) => node.id)).toEqual([
      "sha256:callee",
      "sha256:caller",
      "sha256:method"
    ]);
    expect(response.edges).toHaveLength(2);
    expect(response.direction).toBe("both");
    expect(response.coverage).toEqual(expect.objectContaining({
      resolved: 2,
      evidenceLayer: "structural_graph",
      partial: true
    }));
    expect(runJson).toHaveBeenCalledTimes(3);
    expect(runJson.mock.calls[0]?.[1]).toEqual([
      "query",
      "--cql",
      expect.stringContaining("n.source.startByte <= 4707"),
      "--param",
      "file=crates/globset/src/glob.rs",
      "--timeout-ms",
      "5000",
      "--max-rows",
      "64",
      "--graph",
      "/repo/compass-out/graph.json",
      "--format",
      "json"
    ]);
    expect(runJson.mock.calls[1]?.[1]?.slice(0, 2)).toEqual([
      "callers",
      "sha256:method"
    ]);
    expect(runJson.mock.calls[2]?.[1]?.slice(0, 2)).toEqual([
      "callees",
      "sha256:method"
    ]);
  });
});
