import { describe, expect, it } from "vitest";
import { normalizeStructuredResult } from "./state";

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
