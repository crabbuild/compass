import { describe, expect, it } from "vitest";
import type { CodeQueryResponse } from "@compass/viewer/contracts/codeQuery";
import { codeQueryGraphViewModel } from "./codeQueryGraph";

const anchor = {
  file: "src/lib.rs",
  startByte: 10,
  endByte: 30,
  startLine: 2,
  startColumn: 0,
  endLine: 4,
  endColumn: 1
};
const evidence = {
  layer: "structural_graph" as const,
  origin: "ast" as const,
  extractor: "compass.languages.rust.universal",
  confidence: "exact" as const,
  anchor,
  rule: null,
  wiringSite: null,
  resolution: "exact" as const,
  candidates: []
};
const result: CodeQueryResponse = {
  schema: "compass.query/1",
  operation: "callers",
  results: [],
  nodes: [
    {
      id: "caller",
      kind: "function",
      roles: [],
      name: "caller",
      qualifiedName: "example::caller",
      language: "rust",
      framework: null,
      source: anchor,
      details: { type: "symbol", data: { signature: "fn caller()", modifiers: [] } },
      evidence: [evidence]
    },
    {
      id: "root",
      kind: "function",
      roles: [],
      name: "root",
      qualifiedName: "example::root",
      language: "rust",
      framework: null,
      source: { ...anchor, startByte: 40, endByte: 70, startLine: 6, endLine: 9 },
      details: { type: "symbol", data: { signature: "fn root()", modifiers: [] } },
      evidence: [evidence]
    }
  ],
  edges: [{
    id: "caller-root",
    source: "caller",
    target: "root",
    kind: "calls",
    relationshipSite: anchor,
    details: { type: "call", data: { dispatch: "static" } },
    evidence: [evidence]
  }],
  files: [],
  paths: [],
  diagnostics: [],
  limits: {
    maxDepth: 8,
    maxNodes: 500,
    maxEdges: 1000,
    maxPaths: 100,
    maxCandidates: 20,
    maxSourceBytes: 1_048_576,
    maxResponseBytes: 8_388_608
  },
  truncated: false
};

describe("focused code query graph", () => {
  it("renders only returned query nodes and relationships", () => {
    const model = codeQueryGraphViewModel(result, "Callers of example::root");

    expect(model.title).toBe("Callers of example::root");
    expect(model.stats).toEqual({
      nodes: 2,
      edges: 1,
      communities: 1,
      aggregated: false
    });
    expect(model.nodes.map((node) => node.id)).toEqual(["caller", "root"]);
    expect(model.nodes[0]).toEqual(expect.objectContaining({
      label: "caller",
      kind: "function",
      language: "rust",
      signature: "fn caller()",
      degree: 1,
      source: anchor,
      codeEvidence: [evidence]
    }));
    expect(model.edges).toEqual([expect.objectContaining({
      id: "caller-root",
      source: "caller",
      target: "root",
      relation: "calls",
      confidence: "extracted",
      relationshipSite: anchor,
      codeEvidence: [evidence]
    })]);
  });

  it("keeps an empty query result focused instead of substituting the overview", () => {
    const model = codeQueryGraphViewModel(
      { ...result, nodes: [], edges: [], diagnostics: [{
        code: "no_match",
        message: "No symbol matched root",
        nodeId: null,
        path: null
      }] },
      "Callers"
    );

    expect(model.nodes).toEqual([]);
    expect(model.edges).toEqual([]);
    expect(model.stats.communities).toBe(0);
  });
});
