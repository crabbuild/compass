import path from "node:path";
import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => ({ workspace: {} }));

import { normalizeScopeFiles } from "./scopeFiles";

describe("normalizeScopeFiles", () => {
  it("publishes deterministic contained paths with a bounded result", () => {
    const root = path.resolve("workspace", "compass");
    const result = normalizeScopeFiles(root, [
      path.join(root, "src", "main.ts"),
      path.join(root, "packages", "api.ts"),
      path.join(root, "src", "main.ts"),
      path.resolve(root, "..", "private.ts")
    ], 1);

    expect(result).toEqual({ files: ["packages/api.ts"], truncated: true });
  });
});
