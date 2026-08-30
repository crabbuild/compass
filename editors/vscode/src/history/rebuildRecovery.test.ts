import { describe, expect, it } from "vitest";
import {
  RevisionGraphRebuildRequired,
  rebuildRequiredMessage,
  withRevisionGraphContext
} from "./rebuildRecovery";

describe("history graph rebuild recovery", () => {
  it("identifies the affected revision from a strict history reader failure", async () => {
    const detail = "invalid graph artifacts: stored graph for revision abc uses an unsupported artifact layout (old.json); rebuild this revision graph with the current Compass version";
    const result = withRevisionGraphContext("abcdef123456", async () => {
      throw new Error(detail);
    });

    await expect(result).rejects.toMatchObject({
      name: "RevisionGraphRebuildRequired",
      commit: "abcdef123456",
      detail
    });
    await expect(result).rejects.toBeInstanceOf(RevisionGraphRebuildRequired);
  });

  it("preserves unrelated failures", async () => {
    const original = new Error("history database is unavailable");
    const result = withRevisionGraphContext("abcdef123456", async () => {
      throw original;
    });

    await expect(result).rejects.toBe(original);
  });

  it("uses concise, actionable copy", () => {
    expect(rebuildRequiredMessage("abcdef123456")).toBe(
      "The stored graph for abcdef123 uses an unsupported format. Rebuild it with the current Compass version, then try again."
    );
  });
});
