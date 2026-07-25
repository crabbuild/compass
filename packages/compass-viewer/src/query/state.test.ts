import { describe, expect, it } from "vitest";
import { normalizeStructuredResult, parseNaturalQueryResult } from "./state";

describe("structured query results", () => {
  it("normalizes consistent object rows into columns", () => {
    expect(normalizeStructuredResult({
      rows: [{ symbol: "run", calls: 3 }, { symbol: "save", calls: 2 }]
    })).toEqual({
      columns: ["symbol", "calls"],
      rows: [["run", "3"], ["save", "2"]]
    });
  });

  it("returns undefined for irregular or non-row payloads", () => {
    expect(normalizeStructuredResult({ rows: [["a"], { name: "b" }] })).toBeUndefined();
    expect(normalizeStructuredResult({ value: 1 })).toBeUndefined();
  });
});

describe("natural-language traversal results", () => {
  it("extracts traversal context and actionable source locations", () => {
    const result = parseNaturalQueryResult([
      "Traversal: BFS depth=2 | Start: ['Pipeline'] | 146 nodes found",
      "",
      "NODE Pipeline [src=caching/util/src/Pipeline.scala loc=L154 community=Pipeline]",
      "NODE .assert() [src=caching/util/src/AssertMacros.scala loc=L32 community=.iassert]",
      "NODE String [src= loc= community=EtcdClient]"
    ].join("\n"));

    expect(result.summary).toEqual({
      strategy: "BFS",
      depth: 2,
      starts: ["Pipeline"],
      total: 146
    });
    expect(result.entries).toEqual([
      {
        kind: "NODE",
        label: "Pipeline",
        community: "Pipeline",
        source: {
          file: "caching/util/src/Pipeline.scala",
          startLine: 154,
          endLine: 154
        }
      },
      {
        kind: "NODE",
        label: ".assert()",
        community: ".iassert",
        source: {
          file: "caching/util/src/AssertMacros.scala",
          startLine: 32,
          endLine: 32
        }
      },
      {
        kind: "NODE",
        label: "String",
        community: "EtcdClient"
      }
    ]);
  });

  it("preserves prose answers without manufacturing graph entries", () => {
    const result = parseNaturalQueryResult(
      "Authentication reaches storage through the repository service."
    );
    expect(result.summary).toBeUndefined();
    expect(result.entries).toEqual([]);
    expect(result.prose).toBe(
      "Authentication reaches storage through the repository service."
    );
  });
});
