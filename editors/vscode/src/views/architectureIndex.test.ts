import { describe, expect, it } from "vitest";
import type { ArchitectureViewModel } from "@compass/viewer/contracts/architecture";
import { ArchitectureIndex, routeId } from "./architectureIndex";

const counts = { production: 2, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0 };
const quality = {
  status: "good" as const,
  metrics: {
    sourceScopes: counts,
    unknownSourceFraction: 0,
    generatedVendorLeakage: 0,
    representedNodeFraction: 1,
    representedRelationshipFraction: 1,
    duplicateNames: 0,
    fallbackNames: 0,
    largestGroupFraction: 0.5,
    unknownRelations: 0,
    unassignedNodes: 0,
    unassignedRelationships: 0
  },
  diagnostics: []
};
const group = (id: string, name: string, rank: number) => ({
  id,
  parentId: null,
  kind: "subsystem" as const,
  rank,
  name: {
    value: name,
    provenance: "path" as const,
    membershipSignature: `signature-${id}`,
    quality: 90,
    evidence: [`path:${name}`]
  },
  ownerKey: `crates/${id}`,
  communityIds: [rank - 1],
  nodeCount: 1,
  relationshipCount: 1,
  neighborCount: 1,
  cohesion: 0.8,
  sourceScopes: { ...counts, production: 1 },
  pinned: false
});

const model: ArchitectureViewModel = {
  schema: "compass.viewer.architecture/1",
  title: "Fixture architecture",
  nodes: [
    { id: "api", label: "ApiHandler", kind: "function", sourceFile: "src/api.ts", sourceScope: "production", scopeReason: "source_path", community: 0 },
    { id: "store", label: "LedgerStore", kind: "struct", sourceFile: "src/store.ts", sourceScope: "production", scopeReason: "source_path", community: 1 },
    { id: "fixture", label: "ApiFixture", kind: "function", sourceFile: "tests/api.test.ts", sourceScope: "test", scopeReason: "test_path", community: 0 }
  ],
  relationships: [
    { id: "r-call", source: "api", target: "store", relation: "calls", relationClass: "execution", confidence: "extracted" },
    { id: "r-type", source: "api", target: "store", relation: "type_of", relationClass: "type", confidence: "inferred" },
    { id: "r-test", source: "fixture", target: "api", relation: "calls", relationClass: "execution", confidence: "extracted" }
  ],
  projections: [
    {
      scope: "production",
      defaultLens: "architecture",
      groups: [group("api-group", "API", 1), group("storage-group", "Storage", 2)],
      memberships: [
        { nodeIndex: 0, groupIndex: 0 },
        { nodeIndex: 1, groupIndex: 1 }
      ],
      routes: [],
      overviewGroupIds: ["api-group", "storage-group"],
      overviewRouteIds: [],
      coverage: {
        admitted: 1,
        internal: 0,
        crossGroup: 1,
        unassigned: 0,
        relationClasses: { execution: 1, dependency: 0, type: 1, structure: 0, contextual: 0, unknown: 0 }
      },
      omissions: {
        totalGroups: 2, shownGroups: 2, omittedGroups: 0,
        representedNodes: 2, omittedNodes: 0,
        representedRelationships: 1, omittedRelationships: 0,
        witnessGroupIds: [], maxOverviewGroups: 24, maxOverviewRoutes: 64
      },
      quality
    },
    {
      scope: "all_code",
      defaultLens: "architecture",
      groups: [
        { ...group("api-group", "API", 1), nodeCount: 2, sourceScopes: { ...counts, production: 1, test: 1 } },
        group("storage-group", "Storage", 2)
      ],
      memberships: [
        { nodeIndex: 0, groupIndex: 0 },
        { nodeIndex: 2, groupIndex: 0 },
        { nodeIndex: 1, groupIndex: 1 }
      ],
      routes: [],
      overviewGroupIds: ["api-group", "storage-group"],
      overviewRouteIds: [],
      coverage: {
        admitted: 2,
        internal: 1,
        crossGroup: 1,
        unassigned: 0,
        relationClasses: { execution: 2, dependency: 0, type: 1, structure: 0, contextual: 0, unknown: 0 }
      },
      omissions: {
        totalGroups: 2, shownGroups: 2, omittedGroups: 0,
        representedNodes: 3, omittedNodes: 0,
        representedRelationships: 2, omittedRelationships: 0,
        witnessGroupIds: [], maxOverviewGroups: 24, maxOverviewRoutes: 64
      },
      quality: { ...quality, metrics: { ...quality.metrics, sourceScopes: { ...counts, test: 1 } } }
    }
  ],
  statistics: { nodes: 3, relationships: 3, communities: 2, extracted: 2, inferred: 1, ambiguous: 0 },
  provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null },
  limits: {
    maxNodes: 250000, maxRelationships: 1000000, maxGroups: 100000, maxRoutes: 250000,
    maxOverviewGroups: 24, maxOverviewRoutes: 64, maxNameCandidates: 12,
    maxNameEvidence: 4, maxDiagnostics: 128, maxOmissionWitnesses: 8
  }
};

describe("ArchitectureIndex", () => {
  it("uses Rust-owned production memberships and typed lenses", () => {
    const index = new ArchitectureIndex(model);
    const production = index.overview("production", "all", "architecture");
    expect(production.statistics.totalNodes).toBe(2);
    expect(production.statistics.visibleRelationships).toBe(1);
    expect(production.routes[0]?.id).toBe(routeId("api-group", "storage-group"));

    const allCode = index.overview("all", "all", "architecture");
    expect(allCode.statistics.totalNodes).toBe(3);
    expect(allCode.groups.find((item) => item.id === "api-group")?.scopes.test).toBe(1);

    const typeLens = index.overview("production", "all", "type");
    expect(typeLens.statistics.visibleRelationships).toBe(1);
    expect(typeLens.lens).toBe("type");
  });

  it("pages group evidence and keeps hidden-scope nodes out of production", () => {
    const index = new ArchitectureIndex(model);
    const production = index.groupPage({
      groupId: "api-group", kind: "symbols", page: 1, pageSize: 100,
      scope: "production", evidence: "all", lens: "architecture"
    });
    expect(production.items.map((item) => item.id)).toEqual(["api"]);
    const all = index.groupPage({
      groupId: "api-group", kind: "symbols", page: 1, pageSize: 100,
      scope: "all", evidence: "all", lens: "architecture"
    });
    expect(all.items.map((item) => item.id)).toEqual(["fixture", "api"]);
  });

  it("searches complete scoped groups and source evidence", () => {
    const index = new ArchitectureIndex(model);
    const result = index.search({
      query: "ledger", page: 1, pageSize: 100,
      scope: "production", evidence: "all", lens: "architecture"
    });
    expect(result.items.some((item) => item.label === "LedgerStore")).toBe(true);
  });
});
