import { describe, expect, it, vi } from "vitest";
import { loadTimeline } from "./timelineClient";

describe("history timeline client", () => {
  it("requests a bounded first page and continues from the last loaded commit", async () => {
    const runJson = vi.fn().mockResolvedValue({
      schema: "compass.history.timeline/1",
      repositoryId: "repo",
      selectedHead: "head",
      historyEnabled: true,
      totalEntries: 200,
      hasMore: true,
      nextCursor: "cursor",
      entries: []
    });
    const session = {
      root: "/repo",
      processes: { runJson }
    };

    await loadTimeline(session as never, { limit: 100 });
    await loadTimeline(session as never, { limit: 100, after: "cursor" });
    await loadTimeline(session as never, { limit: 1, revision: "selected" });

    expect(runJson.mock.calls.map((call) => call[1])).toEqual([
      ["history", "timeline", "--limit", "100", "--format", "json"],
      ["history", "timeline", "--limit", "100", "--after", "cursor", "--format", "json"],
      ["history", "timeline", "--rev", "selected", "--limit", "1", "--format", "json"]
    ]);
  });
});
