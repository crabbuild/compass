import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CODE_QUERY_CONTRACT_MANIFEST,
  CodeQueryResponseSchema,
  decodeCodeQueryResponse
} from "./codeQuery";

const contracts = resolve(process.cwd(), "../../fixtures/contracts");

function example(): unknown {
  return JSON.parse(readFileSync(`${contracts}/compass-query-v1.example.json`, "utf8"));
}

describe("compass.query/1", () => {
  it("decodes the checked-in Rust example and fingerprints the shared manifest", () => {
    expect(decodeCodeQueryResponse(example()).schema).toBe("compass.query/1");
    const manifestBytes = readFileSync(`${contracts}/compass-query-v1.manifest.json`);
    expect(JSON.parse(manifestBytes.toString("utf8"))).toEqual(CODE_QUERY_CONTRACT_MANIFEST);
    const expected = readFileSync(
      `${contracts}/compass-query-v1.fingerprint`,
      "utf8"
    ).trim();
    expect(`sha256:${createHash("sha256").update(manifestBytes).digest("hex")}`).toBe(expected);
  });

  it("rejects unknown variants, fields, and unsafe source anchors", () => {
    const unknownOperation = { ...(example() as object), operation: "future" };
    expect(CodeQueryResponseSchema.safeParse(unknownOperation).success).toBe(false);
    const unknownField = { ...(example() as object), unexpected: true };
    expect(CodeQueryResponseSchema.safeParse(unknownField).success).toBe(false);
    const unsafe = {
      ...(example() as Record<string, unknown>),
      nodes: [{
        id: "n",
        kind: "function",
        roles: [],
        name: "n",
        qualifiedName: "n",
        language: null,
        framework: null,
        source: {
          file: "../secret",
          startByte: 0,
          endByte: 1,
          startLine: 1,
          startColumn: 0,
          endLine: 1,
          endColumn: 1
        },
        details: null,
        evidence: []
      }]
    };
    expect(CodeQueryResponseSchema.safeParse(unsafe).success).toBe(false);
  });

  it("retains heuristic wiring and ambiguous candidates", () => {
    const value = example() as Record<string, unknown>;
    value.edges = [{
      id: "e",
      source: "a",
      target: "b",
      kind: "routes_to",
      relationshipSite: null,
      details: {
        type: "route",
        data: { stage: "middleware", position: 1, operation: "GET" }
      },
      evidence: [{
        layer: "structural_graph",
        origin: "heuristic",
        extractor: "express.routes",
        confidence: "ambiguous",
        anchor: null,
        rule: "middleware-chain",
        wiringSite: {
          file: "src/routes.ts",
          startByte: 10,
          endByte: 20,
          startLine: 2,
          startColumn: 0,
          endLine: 2,
          endColumn: 10
        },
        resolution: "ambiguous",
        candidates: [{
          nodeId: "b",
          reason: "matching handler name",
          confidence: "ambiguous"
        }]
      }]
    }];
    const decoded = decodeCodeQueryResponse(value);
    expect(decoded.edges[0]?.evidence[0]?.candidates[0]?.nodeId).toBe("b");
  });

  it("decodes directed-path mismatch diagnostics", () => {
    const value = example() as Record<string, unknown>;
    value.diagnostics = [{
      code: "direction_mismatch",
      message: "A trail exists only from the target back to the source.",
      nodeId: null,
      path: null
    }];
    expect(decodeCodeQueryResponse(value).diagnostics[0]?.code).toBe("direction_mismatch");
  });
});
