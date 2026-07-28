import { describe, expect, it } from "vitest";
import {
  formatGraphEdgeLabel,
  shouldShowGraphEdgeLabel
} from "./edgeLabels";

describe("formatGraphEdgeLabel", () => {
  it.each([
    [{ relation: "contains", confidence: "extracted" }, "contains [EXTRACTED]"],
    [{ relation: "calls", confidence: "inferred" }, "calls [INFERRED]"],
    [{ relation: "references", confidence: "ambiguous" }, "references [AMBIGUOUS]"],
    [
      {
        relation: "2 cross-community edges",
        confidence: "aggregated"
      },
      "2 cross-community edges [AGGREGATED]"
    ],
    [{ relation: "contains" }, "contains"],
    [{ relation: "", confidence: "extracted" }, "[EXTRACTED]"],
    [{ relation: "" }, ""]
  ] as const)("formats %o as %s", (edge, expected) => {
    expect(formatGraphEdgeLabel(edge)).toBe(expected);
  });

  it("keeps route stages and enterprise relations explicit", () => {
    expect(formatGraphEdgeLabel({
      relation: "routes_to",
      details: {
        type: "route",
        data: { stage: "middleware", position: 1, operation: "GET" }
      }
    })).toBe("routes to · middleware 2 · GET");
    expect(formatGraphEdgeLabel({ relation: "publishes" })).toBe("publishes");
    expect(formatGraphEdgeLabel({ relation: "maps_to" })).toBe("maps to");
  });
});

describe("shouldShowGraphEdgeLabel", () => {
  const edge = { id: "e1" };

  it("keeps an edge hidden until it is directly hovered", () => {
    expect(shouldShowGraphEdgeLabel(edge, {
      hoveredEdgeId: null
    })).toBe(false);
    expect(shouldShowGraphEdgeLabel(edge, {
      hoveredEdgeId: "e2"
    })).toBe(false);
  });

  it("reveals only the directly hovered edge", () => {
    expect(shouldShowGraphEdgeLabel(edge, {
      hoveredEdgeId: "e1"
    })).toBe(true);
  });
});
