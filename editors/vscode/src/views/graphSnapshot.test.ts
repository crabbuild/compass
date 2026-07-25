import { access, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CurrentGraphSnapshot } from "./graphSnapshot";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(
    (directory) => rm(directory, { recursive: true, force: true })
  ));
});

describe("CurrentGraphSnapshot", () => {
  it("keeps overview and details on one immutable generation and cleans replacements", async () => {
    const source = await mkdtemp(path.join(tmpdir(), "compass-graph-source-test-"));
    temporaryDirectories.push(source);
    const sourceGraph = path.join(source, "graph.json");
    await writeFile(sourceGraph, "generation-one");
    await writeFile(path.join(source, ".compass_analysis.json"), "analysis-one");

    const snapshot = new CurrentGraphSnapshot();
    const first = await snapshot.replace(sourceGraph);
    await writeFile(sourceGraph, "generation-two");
    expect(await readFile(first, "utf8")).toBe("generation-one");

    const second = await snapshot.replace(sourceGraph);
    await expect(access(first)).rejects.toThrow();
    expect(await readFile(second, "utf8")).toBe("generation-two");

    await snapshot.dispose();
    await expect(access(second)).rejects.toThrow();
  });
});
