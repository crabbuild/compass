import { describe, expect, it } from "vitest";
import type { HistoryHostMessage } from "./panelMessages";
import { historyOperationFor } from "./panelMessages";

describe("history panel messages", () => {
  it("labels timeline and enablement operations", () => {
    expect(historyOperationFor({ type: "retryTimeline" })).toBe("Load history");
    expect(historyOperationFor({ type: "loadMoreTimeline" })).toBe("Load more history");
    expect(historyOperationFor({ type: "enableHistory" })).toBe("Enable history");
  });

  it("supports recoverable bootstrap failures", () => {
    const message: HistoryHostMessage = {
      type: "bootstrapError",
      message: "Git history is unavailable"
    };
    expect(message.type).toBe("bootstrapError");
  });
});
