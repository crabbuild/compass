import { describe, expect, it } from "vitest";
import { formatGraphEdgeLabel } from "./edgeLabels";

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

  it("adds the relationship source line when Compass recorded one", () => {
    expect(formatGraphEdgeLabel({
      relation: "calls",
      confidence: "inferred",
      relationshipSite: {
        file: "src/main.rs",
        startLine: 42,
        endLine: 42
      }
    })).toBe("calls [INFERRED] · src/main.rs:42");
    expect(formatGraphEdgeLabel({
      relation: "calls",
      confidence: "extracted",
      relationshipSite: {
        file: "src/main.rs",
        startLine: 42,
        endLine: 45
      }
    })).toBe("calls [EXTRACTED] · src/main.rs:42–45");
  });
});
