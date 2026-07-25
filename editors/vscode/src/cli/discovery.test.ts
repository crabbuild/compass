import path from "node:path";
import { mkdir, writeFile } from "node:fs/promises";
import { chmodSync } from "node:fs";
import { afterEach, describe, expect, it } from "vitest";

import { discoverCompass } from "./discovery";

const created: string[] = [];
afterEach(async () => {
  const { rm } = await import("node:fs/promises");
  await Promise.all(created.splice(0).map((directory) => rm(directory, {
    recursive: true,
    force: true
  })));
});

describe("discoverCompass", () => {
  it("prefers the configured executable", async () => {
    const directory = path.join(process.cwd(), `.tmp-discovery-${Date.now()}`);
    created.push(directory);
    await mkdir(directory);
    const executable = path.join(directory, "compass");
    await writeFile(executable, "#!/bin/sh\n");
    chmodSync(executable, 0o755);
    const result = await discoverCompass({ get: () => executable });
    expect(result).toEqual({ kind: "found", executable });
  });

  it("falls back to PATH when the configured executable is unavailable", async () => {
    const directory = path.join(process.cwd(), `.tmp-discovery-fallback-${Date.now()}`);
    created.push(directory);
    await mkdir(directory);
    const executable = path.join(directory, "compass");
    await writeFile(executable, "#!/bin/sh\n");
    chmodSync(executable, 0o755);

    const result = await discoverCompass(
      { get: () => path.join(directory, "missing-compass") },
      { PATH: directory },
      "darwin"
    );

    expect(result).toEqual({ kind: "found", executable });
  });

  it("keeps a working configured executable ahead of PATH", async () => {
    const directory = path.join(process.cwd(), `.tmp-discovery-priority-${Date.now()}`);
    const pathDirectory = path.join(directory, "path");
    created.push(directory);
    await mkdir(pathDirectory, { recursive: true });
    const configured = path.join(directory, "configured-compass");
    const fromPath = path.join(pathDirectory, "compass");
    await Promise.all([
      writeFile(configured, "#!/bin/sh\n"),
      writeFile(fromPath, "#!/bin/sh\n")
    ]);
    chmodSync(configured, 0o755);
    chmodSync(fromPath, 0o755);

    const result = await discoverCompass(
      { get: () => configured },
      { PATH: pathDirectory },
      "darwin"
    );

    expect(result).toEqual({ kind: "found", executable: configured });
  });
});
