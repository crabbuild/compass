import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { resolveSource } from "./sourceResolution";

describe("code query source resolution", () => {
  it("rejects repository mismatches, traversal, and escaping symlinks", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "compass-source-resolution-"));
    const repository = path.join(root, "repository");
    const outside = path.join(root, "outside.ts");
    await mkdir(path.join(repository, "src"), { recursive: true });
    await writeFile(path.join(repository, "src/inside.ts"), "inside");
    await writeFile(outside, "outside");
    let symlinkCreated = true;
    try {
      await symlink(outside, path.join(repository, "src/link.ts"));
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code === "EPERM" || code === "EACCES") {
        symlinkCreated = false;
      } else if (code !== "EEXIST") {
        throw error;
      }
    }
    try {
      expect(await resolveSource(
        { id: "repo", root: repository },
        "other",
        "src/inside.ts"
      )).toEqual({ kind: "repository-mismatch" });
      expect((await resolveSource(
        { id: "repo", root: repository },
        "repo",
        "../outside.ts"
      )).kind).toBe("outside-repository");
      if (symlinkCreated) {
        expect((await resolveSource(
          { id: "repo", root: repository },
          "repo",
          "src/link.ts"
        )).kind).toBe("outside-repository");
      }
      expect((await resolveSource(
        { id: "repo", root: repository },
        "repo",
        "src/inside.ts"
      )).kind).toBe("ok");
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
