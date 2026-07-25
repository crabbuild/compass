import { constants } from "node:fs";
import { access } from "node:fs/promises";
import path from "node:path";
import type { WorkspaceConfiguration } from "vscode";

export type CompassDiscovery =
  | { kind: "found"; executable: string }
  | { kind: "missing"; searched: string[] };

export async function discoverCompass(
  configuration: Pick<WorkspaceConfiguration, "get">,
  environment: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform
): Promise<CompassDiscovery> {
  const configured = configuration.get<string>("cliPath")?.trim();
  const candidates = configured
    ? [configured]
    : (environment.PATH ?? "")
      .split(path.delimiter)
      .filter(Boolean)
      .flatMap((directory) => platform === "win32"
        ? ["compass.exe", "compass.cmd", "compass.bat"].map((name) => path.join(directory, name))
        : [path.join(directory, "compass")]);
  for (const candidate of candidates) {
    try {
      await access(candidate, constants.X_OK);
      return { kind: "found", executable: path.resolve(candidate) };
    } catch {
      // Continue through every explicit PATH entry.
    }
  }
  return { kind: "missing", searched: candidates };
}
