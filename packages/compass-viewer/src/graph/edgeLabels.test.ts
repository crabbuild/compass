import { describe, expect, it } from "vitest";
import {
  EDGE_LABEL_ZOOM_THRESHOLD,
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
    hoveredEdgeId: null,
    zoomScale: 1
  };

  it("keeps an unrelated edge hidden in the default wide view", () => {
    expect(shouldShowGraphEdgeLabel(edge, hidden)).toBe(false);
  });

  it("reveals labels through each adaptive interaction", () => {
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
      focusedNodeId: "a"
    })).toBe(true);
    expect(shouldShowGraphEdgeLabel(edge, {
      ...hidden,
      focusedNodeId: "b"
    })).toBe(true);
    expect(shouldShowGraphEdgeLabel(edge, {
      ...hidden,
      zoomScale: EDGE_LABEL_ZOOM_THRESHOLD
    })).toBe(true);
  });
});
