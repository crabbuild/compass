import { describe, expect, it } from "vitest";
import {
  edgeSemanticCategory,
  nodeSemanticCategory,
  nodeSemanticShape
} from "./semanticAppearance";

describe("semantic graph appearance", () => {
  it("groups node kinds into a compact visual vocabulary", () => {
    expect(nodeSemanticCategory("method")).toBe("callable");
    expect(nodeSemanticCategory("type alias")).toBe("type");
    expect(nodeSemanticCategory("FILE")).toBe("module");
    expect(nodeSemanticCategory("http-route")).toBe("other");
    expect(nodeSemanticCategory("route")).toBe("boundary");
    expect(nodeSemanticCategory("variable")).toBe("other");
  });

  it("groups relationships without losing confidence styling", () => {
    expect(edgeSemanticCategory("calls")).toBe("execution");
    expect(edgeSemanticCategory("depends on")).toBe("dependency");
    expect(edgeSemanticCategory("member-of")).toBe("structure");
    expect(edgeSemanticCategory("publishes")).toBe("flow");
    expect(edgeSemanticCategory("cochanges_with")).toBe("other");
  });

  it("provides a second, non-color cue for node categories", () => {
    expect(nodeSemanticShape("callable")).toBe("dot");
    expect(nodeSemanticShape("type")).toBe("diamond");
    expect(nodeSemanticShape("module")).toBe("square");
    expect(nodeSemanticShape("boundary")).toBe("triangle");
  });
});
