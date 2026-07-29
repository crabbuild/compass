import { describe, expect, it } from "vitest";
import type { CallflowViewModel } from "../contracts/callflow";
import {
  filterSectionCalls,
  searchArchitecture,
  sortCalls
} from "./state";

const model: CallflowViewModel = {
  schema: "compass.viewer.callflow/1",
  legacyAggregateOnly: false,
  title: "Fixture",
  sections: [
    {
      id: "api",
      name: "API",
      communities: [],
      nodeCount: 3,
      internalCallCount: 2,
      nodes: [
        {
          id: "a", label: "authenticate", kind: "function",
          sourceFile: "src/api.ts", scope: "production"
        },
        {
          id: "b", label: "database", kind: "function",
          sourceFile: "src/db.ts", scope: "production"
        },
        {
          id: "c", label: "cache", kind: "function",
          sourceFile: "src/cache.ts", scope: "production"
        }
      ],
      edges: [
        { source: "a", target: "b", relation: "calls", confidence: "extracted" },
        { source: "a", target: "c", relation: "calls", confidence: "inferred" }
      ]
    },
    {
      id: "storage",
      name: "Storage",
      communities: [],
      nodeCount: 1,
      internalCallCount: 0,
      nodes: [
        {
          id: "d", label: "database adapter", kind: "class",
          sourceFile: "src/store.ts", scope: "production"
        }
      ],
      edges: []
    }
  ],
  overviewLinks: [],
  crossSectionCalls: [],
  coverage: { internal: 2, crossSection: 0, unassigned: 0 },
  reportHighlights: [],
  statistics: {
    nodes: 4,
    edges: 2,
    communities: 2,
    hyperedges: 0,
    extracted: 1,
    inferred: 1,
    ambiguous: 0
  },
  provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
};

describe("Architecture state", () => {
  it("groups global symbol and call matches by subsystem", () => {
    const groups = searchArchitecture(model, "database");
    expect(groups.map((group) => group.sectionName)).toEqual(["API", "Storage"]);
    expect(groups.flatMap((group) => group.results).map((result) => result.kind))
      .toEqual(expect.arrayContaining(["symbol", "call"]));
  });

  it("filters calls by resolved caller and callee labels", () => {
    const names = new Map([["a", "authenticate"], ["b", "database"]]);
    expect(filterSectionCalls(model.sections[0]!, names, "database")).toHaveLength(1);
  });

  it("sorts call labels without mutating the source", () => {
    const section = model.sections[0]!;
    const source = [...section.edges];
    const names = new Map([
      ["a", "authenticate"],
      ["b", "database"],
      ["c", "cache"]
    ]);
    const sorted = sortCalls(section.edges, names, {
      column: "callee",
      direction: "ascending"
    });
    expect(section.edges).toEqual(source);
    expect(sorted.map((edge) => names.get(edge.target))).toEqual(["cache", "database"]);
  });
});
