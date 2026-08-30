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
        detailAvailable: false,
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
      detailAvailable: false,
      learningStatus: "preferred",
      learningStale: false
    });
  });

  it("preserves aggregated overview edge confidence", () => {
    const parsed = GraphViewModelSchema.parse({
      schema: "compass.viewer.graph/1",
      title: "Aggregate",
      stats: { nodes: 2, edges: 1, communities: 2, aggregated: true },
      nodes: [
        { id: "0", label: "Core", community: 0 },
        { id: "1", label: "Data", community: 1 }
      ],
      edges: [{
        id: "aggregate-edge",
        source: "0",
        target: "1",
        relation: "2 cross-community edges",
        weight: 2,
        confidence: "aggregated"
      }],
      communities: [
        { id: 0, label: "Core", color: "#4E79A7" },
        { id: 1, label: "Data", color: "#F28E2B" }
      ]
    });

    expect(parsed.edges[0]?.confidence).toBe("aggregated");
    expect(parsed.edges[0]?.weight).toBe(2);
  });

  it("preserves an optional relationship source anchor", () => {
    const parsed = GraphViewModelSchema.parse({
      schema: "compass.viewer.graph/1",
      title: "Relationships",
      stats: { nodes: 2, edges: 1, communities: 1, aggregated: false },
      nodes: [
        { id: "caller", label: "caller", community: 0 },
        { id: "callee", label: "callee", community: 0 }
      ],
      edges: [{
        id: "caller-callee",
        source: "caller",
        target: "callee",
        relation: "calls",
        confidence: "inferred",
        relationshipSite: { file: "src/main.rs", startLine: 42, endLine: 42 }
      }],
      communities: [{ id: 0, label: "Core", color: "#4E79A7" }]
    });

    expect(parsed.edges[0]?.relationshipSite).toEqual({
      file: "src/main.rs",
      startLine: 42,
      endLine: 42
    });
  });

  it("validates exact Agent Graph context, challenges, and bounded Retractions", () => {
    const digest = "a".repeat(64);
    const parsed = GraphViewModelSchema.parse({
      schema: "compass.viewer.graph/1",
      title: "Grounded overlay",
      stats: { nodes: 1, edges: 0, communities: 1, aggregated: false },
      nodes: [{
        id: "base-node",
        label: "Base node",
        community: 0,
        challenged: true,
        challenge: {
          challenge: "challenge:review",
          targetId: "base-node",
          effect: "flag",
          masked: false,
          certificateDigest: digest,
          summary: "The exact source contradicts this fact."
        }
      }],
      edges: [],
      communities: [{ id: 0, label: "Core", color: "#4E79A7" }],
      effectiveGraph: {
        effectiveIdentity: digest,
        baseGeneration: { generationId: "generation-1", graphDigest: digest },
        overlayRevision: digest,
        compositionProfile: "augment",
        retractions: {
          total: 1,
          examples: [{
            kind: "assertion",
            id: "assertion:old",
            reasonCode: "superseded",
            explanation: "Replaced with stronger evidence.",
            sequence: 2
          }],
          omittedExamples: 0
        },
        omissions: {
          total: 0,
          direct: 0,
          cascaded: 0,
          examples: [],
          omittedExamples: 0
        }
      }
    });

    expect(parsed.nodes[0]?.challenge?.summary).toContain("contradicts");
    expect(parsed.effectiveGraph?.retractions.examples).toHaveLength(1);
  });

  it("accepts bounded document OCR provenance and geometry", () => {
    const parsed = GraphViewModelSchema.parse({
      schema: "compass.viewer.graph/1",
      title: "Documents",
      stats: { nodes: 2, edges: 1, communities: 1, aggregated: false },
      nodes: [
        {
          id: "report",
          label: "report.pdf",
          community: 0,
          document: {
            role: "root",
            format: "pdf",
            visualCoverage: "partial",
            ocrMode: "auto",
            complete: false,
            ocrProfile: { profile: "pp-ocrv6-small" }
          }
        },
        {
          id: "region",
          label: "Invoice total",
          community: 0,
          document: {
            role: "block",
            kind: "paragraph",
            text: "Invoice total",
            origin: {
              kind: "ocr",
              profile: { profile: "pp-ocrv6-small" },
              confidence_bps: 9234
            },
            locator: {
              kind: "ocr",
              owner: { kind: "pdf", page: 4, item: 1 },
              candidate_id: "page-4",
              width: 1000,
              height: 800,
              polygon: [{ x: 80, y: 100 }, { x: 390, y: 100 }, { x: 390, y: 160 }]
            }
          }
        }
      ],
      edges: [],
      communities: [{ id: 0, label: "Documents", color: "#4E79A7" }]
    });

    expect(parsed.nodes[1]?.document?.origin).toMatchObject({
      kind: "ocr",
      confidence_bps: 9234
    });
    expect(parsed.nodes[1]?.document?.locator).toMatchObject({
      kind: "ocr",
      owner: { kind: "pdf", page: 4 }
    });
  });
});
