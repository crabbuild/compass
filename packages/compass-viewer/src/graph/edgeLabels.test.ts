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
});

describe("shouldShowGraphEdgeLabel", () => {
  const edge = { id: "e1", source: "a", target: "b" };
  const hidden = {
    forceLabels: false,
    focusedNodeId: null,
    focusedEdgeId: null,
    hoveredEdgeId: null
  };

  it("keeps an unrelated edge hidden in the default wide view", () => {
    expect(shouldShowGraphEdgeLabel(edge, hidden)).toBe(false);
  });

  it("reveals labels through explicit and focused interactions", () => {
    expect(shouldShowGraphEdgeLabel(edge, {
      ...hidden,
      forceLabels: true
    })).toBe(true);
    expect(shouldShowGraphEdgeLabel(edge, {
      ...hidden,
      hoveredEdgeId: "e1"
    })).toBe(true);
    expect(shouldShowGraphEdgeLabel(edge, {
      ...hidden,
      focusedEdgeId: "e1"
    })).toBe(true);
    expect(shouldShowGraphEdgeLabel(edge, {
      ...hidden,
      focusedNodeId: "a"
    })).toBe(true);
    expect(shouldShowGraphEdgeLabel(edge, {
      ...hidden,
      focusedNodeId: "b"
    })).toBe(true);
  });

  it("does not reveal unrelated edges as the viewport changes", () => {
    expect(shouldShowGraphEdgeLabel(edge, {
      ...hidden,
      focusedEdgeId: "e2"
    })).toBe(false);
  });
});
