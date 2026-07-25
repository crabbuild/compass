import { describe, expect, it } from "vitest";
import { buildHistoryArgs } from "./buildArguments";

describe("history build arguments", () => {
  it("supports configured, code-only, and inherited profiles with JSONL events", () => {
    expect(buildHistoryArgs({
      revision: "abc",
      all: false,
      firstParent: false,
      profile: { kind: "from", source: "parent" }
    })).toEqual([
      "history", "build", "abc", "--profile-from", "parent",
      "--format", "json", "--events", "jsonl"
    ]);
    expect(buildHistoryArgs({
      revision: "abc",
      all: false,
      firstParent: false,
      profile: { kind: "code-only" }
    })).toContain("--code-only");
  });
});
