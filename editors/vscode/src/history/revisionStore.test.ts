import { mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RepositorySession } from "../workspace/repositorySession";
import { RevisionStore } from "./revisionStore";

const temporaryDirectories: string[] = [];

function envelope(realization: string) {
  return {
    schema: "compass.history.viewer_graph/1",
    commit: "commit",
    realization,
    fingerprint: `fingerprint-${realization}`,
    graph: {
      schema: "compass.viewer.graph/1",
      title: "Fixture",
      stats: { nodes: 1, edges: 0, communities: 1, aggregated: false },
      nodes: [{ id: "node", label: "Node", community: 7 }],
      edges: [],
      communities: [{ id: 7, label: "Core", color: "#4e79a7", hidden: false }],
      hyperedges: []
    }
  };
}

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(
    (directory) => rm(directory, { recursive: true, force: true })
  ));
});

describe("RevisionStore", () => {
  it("uses exact Compass arguments, cleans temporary files, and rejects identity drift", async () => {
    const directory = await mkdtemp(path.join(tmpdir(), "compass-revision-store-test-"));
    temporaryDirectories.push(directory);
    let realization = "r1";
    const run = vi.fn(async (_root: string, args: string[]) => {
      const output = args[args.indexOf("--output") + 1];
      if (!output) throw new Error("missing output");
      await writeFile(output, JSON.stringify(envelope(realization)));
      return { code: 0, stdout: "", stderr: "" };
    });
    const session = {
      root: "/repository",
      processes: { run }
    } as unknown as RepositorySession;
    const store = new RevisionStore(directory, session);
    await store.initialize();

    const overview = await store.load("commit", 8000);
    await store.loadCommunity("commit", 7, 8000, overview);
    expect(run.mock.calls[1]?.[1]).toEqual([
      "history",
      "export",
      "commit",
      "--format",
      "json",
      "--node-limit",
      "8000",
      "--output",
      expect.stringMatching(/\.tmp$/),
      "--community",
      "7"
    ]);
    expect(await readdir(directory)).toEqual([]);

    realization = "r2";
    await expect(store.loadCommunity("commit", 8, 8000, overview))
      .rejects.toThrow("preferred historical realization changed");
    const refreshed = await store.load("commit", 8000);
    expect(refreshed.realization).toBe("r2");
    expect(await readdir(directory)).toEqual([]);
  });
});
