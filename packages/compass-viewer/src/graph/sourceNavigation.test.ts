import { describe, expect, it } from "vitest";
import type { GraphNode } from "../contracts/graph";
import { navigableSource } from "./sourceNavigation";

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
