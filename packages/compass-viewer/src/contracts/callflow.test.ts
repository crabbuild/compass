import { describe, expect, it } from "vitest";
import { CallflowViewModelSchema } from "./callflow";

describe("CallflowViewModelSchema", () => {
  it("normalizes an original schema-v1 payload for the architecture index", () => {
    const model = CallflowViewModelSchema.parse({
      schema: "compass.viewer.callflow/1",
      title: "Legacy",
      sections: [{
        id: "api",
        name: "API",
        communities: [],
        nodes: [{
          id: "handler",
          label: "handler",
          kind: "function",
          sourceFile: null
        }],
        edges: []
      }],
      overviewLinks: [],
      reportHighlights: [],
      statistics: {
        nodes: 1,
        edges: 0,
        communities: 1,
        hyperedges: 0,
        extracted: 0,
        inferred: 0,
        ambiguous: 0
      },
      provenance: { projectName: "Legacy", builtAtCommit: null, generatedAt: null }
    });

    expect(model).toMatchObject({
      schema: "compass.viewer.callflow/1",
      legacyAggregateOnly: true,
      crossSectionCalls: [],
      coverage: { internal: 0, crossSection: 0, unassigned: 0 }
    });
    expect(model.sections[0]).toMatchObject({
      nodeCount: 1,
      internalCallCount: 0
    });
    expect(model.sections[0]!.nodes[0]!.scope).toBe("production");
  });

  it("retains complete additive fields under schema v1", () => {
    const model = CallflowViewModelSchema.parse({
      schema: "compass.viewer.callflow/1",
      title: "Complete",
      sections: [],
      overviewLinks: [],
      crossSectionCalls: [],
      coverage: { internal: 0, crossSection: 0, unassigned: 0 },
      reportHighlights: [],
      statistics: {
        nodes: 0,
        edges: 0,
        communities: 0,
        hyperedges: 0,
        extracted: 0,
        inferred: 0,
        ambiguous: 0
      },
      provenance: { projectName: "Complete", builtAtCommit: null, generatedAt: null }
    });

    expect(model.legacyAggregateOnly).toBe(false);
  });
});
