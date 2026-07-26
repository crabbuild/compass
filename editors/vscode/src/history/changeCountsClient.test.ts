import { describe, expect, it, vi } from "vitest";
import { loadChangeCounts } from "./changeCountsClient";

describe("history change-counts client", () => {
  it("requests counts against the exact parent selected for comparison", async () => {
    const runJson = vi.fn().mockResolvedValue({
      schema: "compass.history.change_counts/1",
      commit: "current",
      parent: "parent-2",
      counts: {
        nodes: { added: 1, removed: 0, changed: 0 },
        edges: { added: 0, removed: 0, changed: 0 },
        hyperedges: { added: 0, removed: 0, changed: 0 }
      }
    });
    const session = {
      root: "/repo",
      processes: { runJson }
    };

    await loadChangeCounts(session as never, "current", "parent-2");

    expect(runJson.mock.calls[0]?.[1]).toEqual([
      "history",
      "change-counts",
      "current",
      "--parent",
      "parent-2",
      "--format",
      "json"
    ]);
  });
});
