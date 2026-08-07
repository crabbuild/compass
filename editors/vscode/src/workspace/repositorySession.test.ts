import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { findPublishedArtifact, resolvePublishedArtifact } from "./repositorySession";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map((directory) =>
    rm(directory, { recursive: true, force: true })
  ));
});

describe("resolvePublishedArtifact", () => {
  it("requires a published snapshot", async () => {
    const output = await fixture();
    expect(findPublishedArtifact(output, "graph.json")).toBeUndefined();
    expect(() => resolvePublishedArtifact(output, "graph.json")).toThrow(/current snapshot/i);
  });

  it("follows the sealed active snapshot", async () => {
    const output = await fixture();
    const snapshot = "snapshot-123";
    await mkdir(path.join(output, "snapshots", snapshot), { recursive: true });
    await writeFile(path.join(output, "current-snapshot"), snapshot);

    expect(resolvePublishedArtifact(output, "graph.json")).toBe(
      path.join(output, "snapshots", snapshot, "graph.json")
    );
  });

  it("rejects malformed and incomplete snapshot pointers", async () => {
    const output = await fixture();
    const snapshot = "snapshot-456";
    const active = path.join(output, "snapshots", snapshot);
    await mkdir(active, { recursive: true });
    await writeFile(path.join(active, "build-incomplete"), "1");
    await writeFile(path.join(output, "current-snapshot"), snapshot);
    expect(() => resolvePublishedArtifact(output, "graph.json")).toThrow(/incomplete/i);

    await writeFile(path.join(output, "current-snapshot"), "../escape");
    expect(() => resolvePublishedArtifact(output, "graph.json")).toThrow(/invalid/i);
  });
});

async function fixture(): Promise<string> {
  const directory = await mkdtemp(path.join(tmpdir(), "compass-vscode-snapshot-"));
  directories.push(directory);
  const output = path.join(directory, "compass-out");
  await mkdir(output);
  return output;
}
