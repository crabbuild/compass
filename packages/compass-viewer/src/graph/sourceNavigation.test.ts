import { describe, expect, it } from "vitest";
import type { GraphNode } from "../contracts/graph";
import { navigableRelationshipSource, navigableSource } from "./sourceNavigation";

function node(source?: GraphNode["source"]): GraphNode {
  return { id: "n1", label: "run", community: 0, source };
}

describe("navigableSource", () => {
  it("accepts file and line metadata", () => {
    const source = { file: "src/main.rs", startLine: 4, endLine: 8 };
    expect(navigableSource(node(source))).toEqual(source);
  });

  it("accepts file and byte metadata", () => {
    const source = { file: "src/main.rs", startByte: 12, endByte: 28 };
    expect(navigableSource(node(source))).toEqual(source);
  });

  it.each([
    undefined,
    { file: "" },
    { file: "   ", startLine: 4 },
    { file: "src/main.rs" }
  ])("rejects incomplete source metadata %#", (source) => {
    expect(navigableSource(node(source))).toBeUndefined();
  });
});

describe("navigableRelationshipSource", () => {
  it("returns only a relationship's own located source anchor", () => {
    const relationshipSite = { file: "src/main.rs", startLine: 19, endLine: 19 };
    expect(navigableRelationshipSource({
      id: "edge",
      source: "caller",
      target: "callee",
      relation: "calls",
      relationshipSite
    })).toEqual(relationshipSite);
    expect(navigableRelationshipSource({
      id: "edge",
      source: "caller",
      target: "callee",
      relation: "calls"
    })).toBeUndefined();
  });
});
