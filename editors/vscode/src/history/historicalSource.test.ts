import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => ({
  Position: class Position {
    constructor(readonly line: number, readonly character: number) {}
  },
  Range: class Range {
    constructor(readonly start: unknown, readonly end: unknown) {}
  },
  Selection: class Selection {
    constructor(readonly start: unknown, readonly end: unknown) {}
  },
  TextEditorRevealType: { InCenterIfOutsideViewport: 0 },
  Uri: {
    from(value: unknown) { return value; }
  },
  workspace: {
    openTextDocument: vi.fn()
  },
  window: {
    showTextDocument: vi.fn()
  }
}));

import {
  historicalSourceArgs,
  repositoryRelativePath
} from "./historicalSource";

describe("historical source paths", () => {
  it("normalizes repository-relative paths and rejects traversal", () => {
    expect(repositoryRelativePath("/repo", "src/core.ts")).toBe("src/core.ts");
    expect(() => repositoryRelativePath("/repo", "../secret.ts"))
      .toThrow("outside the repository");
    expect(() => repositoryRelativePath("/repo", "/tmp/secret.ts"))
      .toThrow("outside the repository");
  });

  it("reads exact commit content with argument-array git invocation", () => {
    expect(historicalSourceArgs("abc1234", "src/core.ts")).toEqual([
      "show",
      "--no-textconv",
      "abc1234:src/core.ts"
    ]);
  });
});
