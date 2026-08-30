import { describe, expect, it } from "vitest";
import { ArchitectureViewModelSchema } from "./architecture";

function model() {
  return {
    schema: "compass.viewer.architecture/1" as const,
    title: "Fixture",
    nodes: [{
      id: "a", label: "A", kind: "function", sourceFile: "src/a.ts",
      sourceScope: "production" as const, scopeReason: "source_path", community: 0
    }],
    relationships: [],
    projections: [{
      scope: "production" as const,
      defaultLens: "architecture" as const,
      groups: [{
        id: "group-a", parentId: null, kind: "subsystem" as const, rank: 1,
        name: {
          value: "Runtime", provenance: "path" as const,
          membershipSignature: "signature-a", quality: 90, evidence: ["path:runtime"]
        },
        ownerKey: "src", communityIds: [0], nodeCount: 1, relationshipCount: 0,
        neighborCount: 0, cohesion: 1, sourceScopes: {
          production: 1, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0
        }, pinned: false
      }],
      memberships: [{ nodeIndex: 0, groupIndex: 0 }],
      routes: [], overviewGroupIds: ["group-a"], overviewRouteIds: [],
      coverage: {
        admitted: 0, internal: 0, crossGroup: 0, unassigned: 0,
        relationClasses: { execution: 0, dependency: 0, type: 0, structure: 0, contextual: 0, unknown: 0 }
      },
      omissions: {
        totalGroups: 1, shownGroups: 1, omittedGroups: 0,
        representedNodes: 1, omittedNodes: 0,
        representedRelationships: 0, omittedRelationships: 0,
        witnessGroupIds: [], maxOverviewGroups: 24, maxOverviewRoutes: 64
      },
      quality: {
        status: "good" as const,
        metrics: {
          sourceScopes: { production: 1, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0 },
          unknownSourceFraction: 0, generatedVendorLeakage: 0,
          representedNodeFraction: 1, representedRelationshipFraction: 1,
          duplicateNames: 0, fallbackNames: 0, largestGroupFraction: 1,
          unknownRelations: 0, unassignedNodes: 0, unassignedRelationships: 0
        }, diagnostics: []
      }
    }],
    statistics: { nodes: 1, relationships: 0, communities: 1, extracted: 0, inferred: 0, ambiguous: 0 },
    provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null },
    limits: {
      maxNodes: 250000, maxRelationships: 1000000, maxGroups: 100000, maxRoutes: 250000,
      maxOverviewGroups: 24, maxOverviewRoutes: 64, maxNameCandidates: 12,
      maxNameEvidence: 4, maxDiagnostics: 128, maxOmissionWitnesses: 8
    }
  };
}

describe("ArchitectureViewModelSchema", () => {
  it("accepts the typed architecture contract", () => {
    expect(ArchitectureViewModelSchema.parse(model()).schema)
      .toBe("compass.viewer.architecture/1");
  });

  it("rejects unknown majors and dangling relationship endpoints", () => {
    expect(() => ArchitectureViewModelSchema.parse({ ...model(), schema: "compass.viewer.architecture/2" }))
      .toThrow();
    const invalid = {
      ...model(),
      relationships: [{
        id: "bad", source: "a", target: "missing", relation: "calls",
        relationClass: "execution", confidence: "extracted"
      }]
    };
    expect(() => ArchitectureViewModelSchema.parse(invalid)).toThrow(/endpoints/);
  });

  it("rejects membership indexes outside their node and group arrays", () => {
    const invalid = model();
    invalid.projections[0]!.memberships[0]!.nodeIndex = 1;
    expect(() => ArchitectureViewModelSchema.parse(invalid)).toThrow(/membership indexes/);
  });
});
