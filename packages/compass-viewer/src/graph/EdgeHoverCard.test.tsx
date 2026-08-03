import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { GraphEdge, GraphNode } from "../contracts/graph";
import { EdgeHoverCard } from "./EdgeHoverCard";

const sourceNode: GraphNode = {
  id: "caller",
  label: "runSearch",
  kind: "function",
  community: 0
};

const targetNode: GraphNode = {
  id: "callee",
  label: "isMatchCandidate",
  kind: "method",
  community: 0
};

const edge: GraphEdge = {
  id: "caller-callee",
  source: sourceNode.id,
  target: targetNode.id,
  relation: "calls",
  confidence: "inferred",
  relationshipSite: {
    file: "crates/globset/src/glob.rs",
    startLine: 143,
    endLine: 143
  },
  codeEvidence: [{
    layer: "program_ir",
    origin: "ast",
    extractor: "rust",
    confidence: "inferred",
    anchor: null,
    rule: "call-expression",
    wiringSite: null,
    resolution: "exact",
    candidates: []
  }]
};

describe("EdgeHoverCard", () => {
  it("explains relationship direction, confidence, evidence, and source", () => {
    const markup = renderToStaticMarkup(
      <EdgeHoverCard
        edge={edge}
        sourceNode={sourceNode}
        targetNode={targetNode}
        hover={{ edgeId: edge.id, x: 100, y: 80 }}
      />
    );

    expect(markup).toContain("runSearch");
    expect(markup).toContain("isMatchCandidate");
    expect(markup).toContain("calls");
    expect(markup).toContain("Inferred");
    expect(markup).toContain("call-expression");
    expect(markup).toContain("crates/globset/src/glob.rs:143");
    expect(markup).toContain("Double-click");
    expect(markup).toContain("to open relationship source");
  });

  it("omits source affordances when an edge has no relationship anchor", () => {
    const markup = renderToStaticMarkup(
      <EdgeHoverCard
        edge={{ ...edge, relationshipSite: undefined, codeEvidence: undefined }}
        sourceNode={sourceNode}
        targetNode={targetNode}
        hover={{ edgeId: edge.id, x: 100, y: 80 }}
      />
    );

    expect(markup).not.toContain("Double-click");
    expect(markup).not.toContain("compass-edge-metadata");
  });

  it("does not invent confidence when the graph did not record it", () => {
    const markup = renderToStaticMarkup(
      <EdgeHoverCard
        edge={{ ...edge, confidence: undefined }}
        sourceNode={sourceNode}
        targetNode={targetNode}
        hover={{ edgeId: edge.id, x: 100, y: 80 }}
      />
    );

    expect(markup).toContain("Unspecified");
    expect(markup).not.toContain(">Inferred<");
  });
});
