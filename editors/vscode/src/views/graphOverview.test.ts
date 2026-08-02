import { mkdir, mkdtemp, rm, stat, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { loadPreparedGraphOverview } from "./graphOverview";

const model = {
  schema: "compass.viewer.graph/1",
  title: "graph.json",
  stats: { nodes: 1, edges: 0, communities: 1, aggregated: false },
  nodes: [{ id: "a", label: "A", community: 0 }],
  edges: [],
  communities: [{ id: 0, label: "Community 0", color: "#4E79A7", hidden: false }],
  hyperedges: []
};

describe("loadPreparedGraphOverview", () => {
  let directory: string | undefined;

  afterEach(async () => {
    if (directory) await rm(directory, { recursive: true, force: true });
    directory = undefined;
  });

  async function fixture({
    sourceGraphBytes = 12,
    nodeLimit = 5000,
    schema = "compass.graph-overview/2"
  }: {
    sourceGraphBytes?: number;
    nodeLimit?: number;
    schema?: string;
  } = {}) {
    directory = await mkdtemp(path.join(tmpdir(), "compass-graph-overview-"));
    const output = path.join(directory, "compass-out");
    await mkdir(output);
    const graphPath = path.join(output, "graph.json");
    await writeFile(graphPath, "x".repeat(12));
    const overviewPath = path.join(output, "graph-overview.json");
    await writeFile(overviewPath, JSON.stringify({
      schema,
      sourceGraphBytes,
      nodeLimit,
      model
    }));
    const graphStat = await stat(graphPath);
    await utimes(overviewPath, graphStat.atime, new Date(graphStat.mtimeMs + 1_000));
    return graphPath;
  }

  it("loads a prepared overview matching the current graph generation and limit", async () => {
    const graphPath = await fixture();

    await expect(loadPreparedGraphOverview(graphPath, 5000)).resolves.toEqual(model);
  });

  it("rejects an overview for a different graph byte count", async () => {
    const graphPath = await fixture({ sourceGraphBytes: 11 });

    await expect(loadPreparedGraphOverview(graphPath, 5000)).resolves.toBeUndefined();
  });

  it("rejects an overview generated with a different node limit", async () => {
    const graphPath = await fixture({ nodeLimit: 2500 });

    await expect(loadPreparedGraphOverview(graphPath, 5000)).resolves.toBeUndefined();
  });

  it("rejects an overview from the renderer without relationship anchors", async () => {
    const graphPath = await fixture({ schema: "compass.graph-overview/1" });

    await expect(loadPreparedGraphOverview(graphPath, 5000)).resolves.toBeUndefined();
  });

  it("rejects an overview older than graph.json", async () => {
    const graphPath = await fixture();
    const overviewPath = path.join(path.dirname(graphPath), "graph-overview.json");
    const graphStat = await stat(graphPath);
    await utimes(overviewPath, graphStat.atime, new Date(graphStat.mtimeMs - 1_000));

    await expect(loadPreparedGraphOverview(graphPath, 5000)).resolves.toBeUndefined();
  });
});
