import { chmodSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { discoverCompass, inspectCompassInstallation } from "./discovery";

const created: string[] = [];
afterEach(async () => {
  const { rm } = await import("node:fs/promises");
  await Promise.all(created.splice(0).map((directory) => rm(directory, {
    recursive: true,
    force: true
  })));
});

describe("discoverCompass", () => {
  it("finds every installed version and keeps the configured executable active", async () => {
    const directory = await temporaryDirectory("versions");
    const pathDirectory = path.join(directory, "path");
    await mkdir(pathDirectory);
    const configured = path.join(directory, "configured-compass");
    const fromPath = path.join(pathDirectory, "compass");
    await Promise.all([
      writeCompass(configured, "0.1.4"),
      writeCompass(fromPath, "0.2.0")
    ]);

    const result = await discoverCompass(
      { get: () => configured },
      { PATH: pathDirectory },
      "darwin",
      { commonDirectories: [] }
    );

    expect(result).toEqual({
      kind: "found",
      executable: configured,
      version: "0.1.4",
      installations: [
        {
          executable: configured,
          version: "0.1.4",
          source: "configured"
        },
        {
          executable: fromPath,
          version: "0.2.0",
          source: "path"
        }
      ],
      searched: [configured, fromPath]
    });
  });

  it("falls back to PATH when the configured executable is unavailable", async () => {
    const directory = await temporaryDirectory("fallback");
    const executable = path.join(directory, "compass");
    await writeCompass(executable, "0.1.5");
    const missing = path.join(directory, "missing-compass");

    const result = await discoverCompass(
      { get: () => missing },
      { PATH: directory },
      "darwin",
      { commonDirectories: [] }
    );

    expect(result).toMatchObject({
      kind: "found",
      executable,
      version: "0.1.5"
    });
    expect(result.installations).toEqual([{
      executable,
      version: "0.1.5",
      source: "path"
    }]);
  });

  it("detects the default install directory even when it is not on PATH", async () => {
    const directory = await temporaryDirectory("common");
    const executable = path.join(directory, "compass");
    await writeCompass(executable, "0.3.1");

    const result = await discoverCompass(
      { get: () => "" },
      { PATH: "" },
      "linux",
      { commonDirectories: [directory] }
    );

    expect(result).toMatchObject({
      kind: "found",
      executable,
      version: "0.3.1",
      installations: [{
        executable,
        version: "0.3.1",
        source: "common"
      }]
    });
  });

  it("deduplicates a configured executable that is also on PATH", async () => {
    const directory = await temporaryDirectory("deduplicate");
    const executable = path.join(directory, "compass");
    await writeCompass(executable, "0.1.5");

    const result = await discoverCompass(
      { get: () => executable },
      { PATH: directory },
      "darwin",
      { commonDirectories: [directory] }
    );

    expect(result.installations).toHaveLength(1);
    expect(result.installations[0]?.source).toBe("configured");
  });

  it("keeps an executable discoverable when its version cannot be read", async () => {
    const directory = await temporaryDirectory("unknown-version");
    const executable = path.join(directory, "compass");
    await writeFile(executable, "#!/bin/sh\nexit 1\n");
    chmodSync(executable, 0o755);

    await expect(inspectCompassInstallation(executable)).resolves.toEqual({
      executable,
      source: "configured"
    });
  });
});

async function temporaryDirectory(label: string): Promise<string> {
  const directory = path.join(
    process.cwd(),
    `.tmp-discovery-${label}-${Date.now()}-${created.length}`
  );
  created.push(directory);
  await mkdir(directory);
  return directory;
}

async function writeCompass(executable: string, version: string): Promise<void> {
  await writeFile(executable, `#!/bin/sh\necho 'compass ${version}'\n`);
  chmodSync(executable, 0o755);
}
