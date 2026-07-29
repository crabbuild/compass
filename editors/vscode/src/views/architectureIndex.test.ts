import { describe, expect, it } from "vitest";
import type { CallflowViewModel } from "@compass/viewer/contracts/callflow";
import { ArchitectureIndex, routeId } from "./architectureIndex";

const model: CallflowViewModel = {
  schema: "compass.viewer.callflow/2",
  title: "Fixture — Architecture Flow",
  sections: [
    {
      id: "overview",
      name: "Architecture Overview",
      communities: [],
      nodeCount: 0,
      internalCallCount: 0,
      nodes: [],
      edges: []
    },
    {
      id: "api",
      name: "API",
      communities: ["0"],
      nodeCount: 3,
      internalCallCount: 1,
      nodes: [
        {
          id: "handler", label: "request_handler", kind: "function",
          sourceFile: "src/api.ts", scope: "production"
        },
        {
          id: "helper", label: "helper", kind: "function",
          sourceFile: "src/helper.ts", scope: "production"
        },
        {
          id: "api_test", label: "request_handler_test", kind: "function",
          sourceFile: "tests/api.test.ts", scope: "test"
        }
      ],
      edges: [
        {
          source: "handler", target: "helper",
          relation: "calls", confidence: "extracted"
        }
      ]
    },
    {
      id: "storage",
      name: "Storage",
      communities: ["1"],
      nodeCount: 2,
      internalCallCount: 0,
      nodes: [
        {
          id: "store", label: "save_record", kind: "function",
          sourceFile: "src/store.ts", scope: "production"
        },
        {
          id: "generated", label: "GeneratedModel", kind: "class",
          sourceFile: "generated/model.ts", scope: "generated"
        }
      ],
      edges: []
    }
  ],
  overviewLinks: [{ sourceSection: "api", targetSection: "storage", calls: 3 }],
  crossSectionCalls: [
    {
      source: "handler", target: "store", sourceSection: "api",
      targetSection: "storage", relation: "calls", confidence: "extracted"
    },
    {
      source: "api_test", target: "store", sourceSection: "api",
      targetSection: "storage", relation: "calls", confidence: "inferred"
    },
    {
      source: "handler", target: "generated", sourceSection: "api",
      targetSection: "storage", relation: "references", confidence: "ambiguous"
    }
  ],
  coverage: { internal: 1, crossSection: 3, unassigned: 0 },
  reportHighlights: [],
  statistics: {
    nodes: 5,
    edges: 4,
    communities: 2,
    hyperedges: 0,
    extracted: 2,
    inferred: 1,
    ambiguous: 1
  },
  provenance: { projectName: "Fixture", builtAtCommit: "abc123", generatedAt: null }
};

describe("ArchitectureIndex", () => {
  it("defaults projections to production while disclosing complete totals", () => {
    const overview = new ArchitectureIndex(model).overview("production", "all");

    expect(overview.statistics).toMatchObject({
      visibleNodes: 3,
      totalNodes: 5,
      visibleCalls: 2,
      totalCalls: 4
    });
    expect(overview.routes).toEqual([
      expect.objectContaining({
        sourceSection: "api",
        targetSection: "storage",
        calls: 1,
        extracted: 1
      })
    ]);
    expect(overview.sections.find((section) => section.id === "api")?.scopes.test).toBe(1);
  });

  it("restores test and generated calls in all-code scope", () => {
    const overview = new ArchitectureIndex(model).overview("all", "all");
    expect(overview.statistics.visibleNodes).toBe(5);
    expect(overview.statistics.visibleCalls).toBe(4);
    expect(overview.routes[0]).toMatchObject({
      calls: 3,
      extracted: 1,
      inferred: 1,
      ambiguous: 1
    });
  });

  it("filters evidence and pages every call behind a route", () => {
    const index = new ArchitectureIndex(model);
    const page = index.routePage({
      routeId: routeId("api", "storage"),
      scope: "all",
      evidence: "all",
      page: 2,
      pageSize: 2
    });
    expect(page).toMatchObject({ total: 3, start: 3, end: 3 });
    expect(page.items).toHaveLength(1);

    const inferred = index.overview("all", "inferred");
    expect(inferred.routes[0]).toMatchObject({ calls: 1, inferred: 1 });
  });

  it("searches the complete retained model rather than one visible page", () => {
    const results = new ArchitectureIndex(model).search({
      query: "GeneratedModel",
      scope: "all",
      evidence: "all",
      page: 1,
      pageSize: 10
    });
    expect(results.items).toEqual(expect.arrayContaining([
      expect.objectContaining({
        kind: "symbol",
        label: "GeneratedModel",
        sectionId: "storage"
      })
    ]));
  });

  it("returns bounded section pages with complete range metadata", () => {
    const page = new ArchitectureIndex(model).sectionPage({
      sectionId: "api",
      kind: "symbols",
      scope: "all",
      evidence: "all",
      page: 1,
      pageSize: 2
    });
    expect(page).toMatchObject({
      kind: "symbols",
      total: 3,
      start: 1,
      end: 2,
      pageCount: 2
    });
    expect(page.items).toHaveLength(2);
  });
});
