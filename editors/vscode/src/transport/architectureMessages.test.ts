import { describe, expect, it } from "vitest";
import {
  ArchitectureToHostMessageSchema,
  HostToArchitectureMessageSchema
} from "./architectureMessages";

describe("architecture messages", () => {
  it("accepts an identity-bound section request", () => {
    expect(ArchitectureToHostMessageSchema.parse({
      type: "requestSection",
      requestId: "request-1",
      repositoryId: "/repo",
      generation: 2,
      scope: "production",
      evidence: "all",
      page: 1,
      pageSize: 100,
      sectionId: "api",
      kind: "calls"
    })).toMatchObject({ sectionId: "api", generation: 2 });
  });

  it("rejects unbounded page sizes", () => {
    expect(ArchitectureToHostMessageSchema.safeParse({
      type: "requestRoute",
      requestId: "request-1",
      repositoryId: "/repo",
      generation: 2,
      scope: "all",
      evidence: "all",
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
        sections: [],
        routes: [],
        statistics: {
          visibleNodes: 2,
          totalNodes: 4,
          visibleCalls: 1,
          totalCalls: 3,
          communities: 2,
          extracted: 2,
          inferred: 1,
          ambiguous: 0
        },
        coverage: { internal: 1, crossSection: 2, unassigned: 0 },
        provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
      }
    });

    expect(parsed.type).toBe("architectureOverview");
    if (parsed.type !== "architectureOverview") throw new Error("Expected overview");
    expect("sections" in parsed.model).toBe(true);
    expect("crossSectionCalls" in parsed.model).toBe(false);
  });
});
