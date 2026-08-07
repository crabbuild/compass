import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import type { WorkspaceFolder } from "vscode";
import { afterEach, describe, expect, it } from "vitest";
import type { CompassProcessManager } from "../cli/processManager";
import { refreshedGraphState, SessionRegistry } from "./sessionRegistry";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map((directory) =>
    rm(directory, { recursive: true, force: true })
  ));
});

describe("refreshedGraphState", () => {
  it("gives an active writer precedence over filesystem state", () => {
    expect(refreshedGraphState("available", true, true)).toBe("building");
    expect(refreshedGraphState("not-materialized", false, true)).toBe("building");
  });

  it("preserves a failed operation until a successful workflow changes it", () => {
    expect(refreshedGraphState("failed", false, false)).toBe("failed");
    expect(refreshedGraphState("failed", true, false)).toBe("failed");
  });

  it("uses graph materialization for stable non-failure states", () => {
    expect(refreshedGraphState("available", false, false)).toBe("not-materialized");
    expect(refreshedGraphState("not-materialized", true, false)).toBe("available");
  });
});

describe("SessionRegistry.refresh", () => {
  it("keeps a fresh repository available for initialization", async () => {
    const root = await fixture();
    const registry = registryFor(root);

    await expect(registry.refresh()).resolves.toBeUndefined();
    expect(registry.all()[0]).toMatchObject({
      graphState: "not-materialized",
      graphError: undefined
    });
  });

  it("refreshes valid and fresh repositories independently", async () => {
    const ready = await fixture();
    const fresh = await fixture();
    await publishGraph(ready, "snapshot-ready");
    const registry = registryFor(ready, fresh);

    await expect(registry.refresh()).resolves.toBeUndefined();
    expect(registry.all().map((session) => session.graphState)).toEqual([
      "available",
      "not-materialized"
    ]);
  });

  it("isolates an invalid pointer and recovers after a valid publication", async () => {
    const root = await fixture();
    const output = path.join(root, "compass-out");
    await mkdir(output);
    await writeFile(path.join(output, "current-snapshot"), "../escape");
    const registry = registryFor(root);

    await expect(registry.refresh()).resolves.toBeUndefined();
    expect(registry.all()[0]).toMatchObject({
      graphState: "failed",
      graphError: expect.stringMatching(/invalid/i)
    });

    await publishGraph(root, "snapshot-recovered");
    await expect(registry.refresh()).resolves.toBeUndefined();
    expect(registry.all()[0]).toMatchObject({
      graphState: "available",
      graphError: undefined
    });
  });
});

async function fixture(): Promise<string> {
  const directory = await mkdtemp(path.join(tmpdir(), "compass-vscode-registry-"));
  directories.push(directory);
  return directory;
}

async function publishGraph(root: string, snapshot: string): Promise<void> {
  const output = path.join(root, "compass-out");
  const active = path.join(output, "snapshots", snapshot);
  await mkdir(active, { recursive: true });
  await writeFile(path.join(active, "graph.json"), "{}");
  await writeFile(path.join(output, "current-snapshot"), snapshot);
}

function registryFor(...roots: string[]): SessionRegistry {
  const folders = roots.map((root, index) => ({
    index,
    name: path.basename(root),
    uri: {
      fsPath: root,
      toString: () => `file://${root}`
    }
  })) as unknown as WorkspaceFolder[];
  return new SessionRegistry(folders, {} as CompassProcessManager);
}
