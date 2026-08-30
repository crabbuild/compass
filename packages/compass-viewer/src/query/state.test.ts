import { describe, expect, it } from "vitest";
import {
  normalizeStructuredResult,
  parseExplanationResult
} from "./state";

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

describe("symbol explanations", () => {
  it("separates node metadata from incoming and outgoing relationships", () => {
    expect(parseExplanationResult([
      "Node: Checkout.run",
      "  ID:        checkout-run",
      "  Source:    src/checkout.rs L12:1-L18:2",
      "  Type:      function",
      "  Community: Checkout",
      "  Degree:    2",
      "",
      "Connections (2):",
      "  --> Database.save [calls] [EXACT] src/checkout.rs:L16",
      "  <-- Api.route [routes_to] [INFERRED] src/api.rs:L8"
    ].join("\n"))).toEqual({
      kind: "node",
      label: "Checkout.run",
      id: "checkout-run",
      source: { file: "src/checkout.rs", startLine: 12, endLine: 18 },
      type: "function",
      community: "Checkout",
      degree: 2,
      connections: [{
        direction: "outgoing",
        label: "Database.save",
        relation: "calls",
        confidence: "EXACT"
      }, {
        direction: "incoming",
        label: "Api.route",
        relation: "routes_to",
        confidence: "INFERRED"
      }]
    });
  });

  it("itemizes ambiguous candidates for a full-ID retry", () => {
    expect(parseExplanationResult([
      "Ambiguous: 'run' matches 2 source-backed nodes.",
      "  src/a.rs L3:1-L3:6",
      "    id: first",
      "  src/b.rs L7:1-L7:6",
      "    id: second",
      "Retry with the full node ID."
    ].join("\n"))).toEqual({
      kind: "ambiguous",
      title: "Ambiguous: 'run' matches 2 source-backed nodes.",
      candidates: [{ id: "first", source: "src/a.rs L3:1-L3:6" }, {
        id: "second",
        source: "src/b.rs L7:1-L7:6"
      }]
    });
  });
});
