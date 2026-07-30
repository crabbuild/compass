import { describe, expect, it } from "vitest";
import { CallflowViewModelSchema } from "./callflow";

describe("CallflowViewModelSchema", () => {
  const completeModel = {
    schema: "compass.viewer.callflow/1" as const,
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
  };

  it("accepts the complete schema-v1 contract", () => {
    expect(CallflowViewModelSchema.parse(completeModel)).toEqual(completeModel);
  });

  it("rejects incomplete schema-v1 payloads", () => {
    expect(() => CallflowViewModelSchema.parse({
      schema: "compass.viewer.callflow/1",
      title: "Incomplete",
      sections: [],
      overviewLinks: [],
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
      provenance: { projectName: "Incomplete", builtAtCommit: null, generatedAt: null }
    })).toThrow();
  });

  it("rejects schema v2", () => {
    expect(() => CallflowViewModelSchema.parse({
      ...completeModel,
      schema: "compass.viewer.callflow/2"
    })).toThrow();
  });
});
