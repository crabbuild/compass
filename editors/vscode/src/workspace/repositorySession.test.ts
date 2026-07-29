import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { resolvePublishedArtifact } from "./repositorySession";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map((directory) =>
    rm(directory, { recursive: true, force: true })
  ));
});

describe("resolvePublishedArtifact", () => {
  it("falls back to the legacy root when no generation is published", async () => {
    const output = await fixture();
    expect(resolvePublishedArtifact(output, "graph.json")).toBe(path.join(output, "graph.json"));
  });

  it("follows the sealed active generation", async () => {
    const output = await fixture();
    const generation = "generation-123";
    await mkdir(path.join(output, ".compass-generations", generation), { recursive: true });
    await writeFile(path.join(output, ".compass-active-generation"), generation);

    expect(resolvePublishedArtifact(output, "graph.json")).toBe(
      path.join(output, ".compass-generations", generation, "graph.json")
    );
  });

  it("rejects malformed and incomplete generation pointers", async () => {
    const output = await fixture();
    const generation = "generation-456";
    const active = path.join(output, ".compass-generations", generation);
    await mkdir(active, { recursive: true });
    await writeFile(path.join(active, ".compass-build-incomplete"), "1");
    await writeFile(path.join(output, ".compass-active-generation"), generation);
    expect(resolvePublishedArtifact(output, "graph.json")).toBe(path.join(output, "graph.json"));

    await writeFile(path.join(output, ".compass-active-generation"), "../escape");
    expect(resolvePublishedArtifact(output, "graph.json")).toBe(path.join(output, "graph.json"));
  });
});

async function fixture(): Promise<string> {
  const directory = await mkdtemp(path.join(tmpdir(), "compass-vscode-generation-"));
  directories.push(directory);
  const output = path.join(directory, "compass-out");
  await mkdir(output);
  return output;
}
