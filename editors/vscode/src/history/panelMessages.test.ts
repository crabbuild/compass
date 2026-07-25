import { describe, expect, it } from "vitest";
import type { HistoryHostMessage } from "./panelMessages";
import { historyOperationFor } from "./panelMessages";

describe("history panel messages", () => {
  it("labels retryTimeline as history loading", () => {
    expect(historyOperationFor({ type: "retryTimeline" })).toBe("Load history");
  });

  it("supports recoverable bootstrap failures", () => {
    const message: HistoryHostMessage = {
      type: "bootstrapError",
      message: "Git history is unavailable"
    };
    expect(message.type).toBe("bootstrapError");
  });
});
