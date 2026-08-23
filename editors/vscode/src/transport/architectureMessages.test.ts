import { describe, expect, it } from "vitest";
import {
  ArchitectureToHostMessageSchema,
  HostToArchitectureMessageSchema
} from "./architectureMessages";

describe("architecture messages", () => {
  it("accepts an identity-bound section request", () => {
    expect(ArchitectureToHostMessageSchema.parse({
      type: "requestGroup",
      requestId: "request-1",
      repositoryId: "/repo",
      generation: 2,
      scope: "production",
      evidence: "all",
      lens: "architecture",
      page: 1,
      pageSize: 100,
      groupId: "api",
      kind: "relationships"
    })).toMatchObject({ groupId: "api", generation: 2 });
  });

  it("rejects unbounded page sizes", () => {
    expect(ArchitectureToHostMessageSchema.safeParse({
      type: "requestRoute",
      requestId: "request-1",
      repositoryId: "/repo",
      generation: 2,
      scope: "all",
      evidence: "all",
      lens: "architecture",
      page: 1,
      pageSize: 101,
      routeId: "api→storage"
    }).success).toBe(false);
  });

  it("validates bounded overview responses without a full-model payload", () => {
    const parsed = HostToArchitectureMessageSchema.parse({
      type: "architectureOverview",
      requestId: "request-1",
      repositoryId: "/repo",
      generation: 2,
      model: {
        title: "Fixture",
        scope: "production",
        evidence: "all",
        lens: "architecture",
        groups: [],
        routes: [],
        statistics: {
          visibleNodes: 2,
          totalNodes: 4,
          visibleRelationships: 1,
          totalRelationships: 3,
          communities: 2,
          extracted: 2,
          inferred: 1,
          ambiguous: 0
        },
        coverage: { internal: 1, crossGroup: 2, unassigned: 0 },
        omissions: {
          totalGroups: 2, shownGroups: 2, omittedGroups: 0,
          representedNodes: 4, omittedNodes: 0,
          representedRelationships: 3, omittedRelationships: 0,
          witnessGroupIds: [], maxOverviewGroups: 24, maxOverviewRoutes: 64
        },
        quality: {
          status: "good",
          metrics: {
            sourceScopes: { production: 4, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0 },
            unknownSourceFraction: 0, generatedVendorLeakage: 0,
            representedNodeFraction: 1, representedRelationshipFraction: 1,
            duplicateNames: 0, fallbackNames: 0, largestGroupFraction: 0.5,
            unknownRelations: 0, unassignedNodes: 0, unassignedRelationships: 0
          }, diagnostics: []
        },
        provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
      }
    });

    expect(parsed.type).toBe("architectureOverview");
    if (parsed.type !== "architectureOverview") throw new Error("Expected overview");
    expect("groups" in parsed.model).toBe(true);
    expect("crossSectionCalls" in parsed.model).toBe(false);
  });
});
