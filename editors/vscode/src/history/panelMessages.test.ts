import { describe, expect, it } from "vitest";
import type { HistoryHostMessage } from "./panelMessages";
import { historyOperationFor } from "./panelMessages";

describe("history panel messages", () => {
  it("labels timeline and enablement operations", () => {
    expect(historyOperationFor({ type: "retryTimeline" })).toBe("Load history");
    expect(historyOperationFor({ type: "loadMoreTimeline" })).toBe("Load more history");
    expect(historyOperationFor({ type: "enableHistory" })).toBe("Enable history");
    expect(historyOperationFor({ type: "compareCommunity" })).toBe("Compare community");
  });

  it("supports recoverable bootstrap failures", () => {
    const message: HistoryHostMessage = {
      type: "bootstrapError",
      message: "Git history is unavailable"
    };
    expect(message.type).toBe("bootstrapError");
  });

  it("carries correlated dual-revision community comparisons", () => {
    const message: HistoryHostMessage = {
      type: "communityComparison",
      requestId: "request",
      commit: "current",
      parent: "parent",
      communityId: 7,
      nodeLimit: 5000,
      currentGraph: {
        schema: "compass.viewer.graph/1",
        title: "Current",
        stats: { nodes: 0, edges: 0, communities: 0, aggregated: false },
        nodes: [],
        edges: [],
        communities: [],
        hyperedges: []
      }
    };

    expect(message).toMatchObject({
      type: "communityComparison",
      requestId: "request",
      parent: "parent",
      communityId: 7
    });
  });
});
