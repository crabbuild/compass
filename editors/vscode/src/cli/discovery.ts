import { execFile } from "node:child_process";
import { constants } from "node:fs";
import { access } from "node:fs/promises";
import path from "node:path";
import type { WorkspaceConfiguration } from "vscode";

export type CompassInstallationSource = "configured" | "path" | "common";

export type CompassInstallation = {
  executable: string;
  version?: string | undefined;
  source: CompassInstallationSource;
};

export type CompassDiscovery =
  | {
    kind: "found";
    executable: string;
    version?: string | undefined;
    installations: CompassInstallation[];
    searched: string[];
  }
  | {
    kind: "missing";
    installations: [];
    searched: string[];
  };

type Candidate = {
  executable: string;
  source: CompassInstallationSource;
};

export type CompassDiscoveryOptions = {
  commonDirectories?: readonly string[] | undefined;
};

export async function discoverCompass(
  configuration: Pick<WorkspaceConfiguration, "get">,
  environment: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  options: CompassDiscoveryOptions = {}
): Promise<CompassDiscovery> {
  const candidates = installationCandidates(
    configuration,
    environment,
    platform,
    options
  );
  const installations = (await Promise.all(candidates.map((candidate) => (
    inspectCompassInstallation(candidate.executable, candidate.source)
  )))).filter((installation) => installation !== undefined);
  const active = installations[0];
  const searched = candidates.map((candidate) => candidate.executable);

  return active
    ? {
      kind: "found",
      executable: active.executable,
      version: active.version,
      installations,
      searched
    }
    : { kind: "missing", installations: [], searched };
}

export async function inspectCompassInstallation(
  executable: string,
  source: CompassInstallationSource = "configured"
): Promise<CompassInstallation | undefined> {
  const resolved = resolveCompassPath(executable);
  try {
    await access(resolved, constants.X_OK);
  } catch {
    return undefined;
  }
  const version = await compassVersion(resolved);
  return {
    executable: resolved,
    ...(version ? { version } : {}),
    source
  };
}

export function resolveCompassPath(
  executable: string,
  environment: NodeJS.ProcessEnv = process.env
): string {
  const trimmed = executable.trim();
  const home = environment.HOME ?? environment.USERPROFILE;
  const expanded = home && (trimmed === "~"
    || trimmed.startsWith("~/")
    || trimmed.startsWith("~\\"))
    ? path.join(home, trimmed.slice(2))
    : trimmed;
  return path.resolve(expanded);
}

function installationCandidates(
  configuration: Pick<WorkspaceConfiguration, "get">,
  environment: NodeJS.ProcessEnv,
  platform: NodeJS.Platform,
  options: CompassDiscoveryOptions
): Candidate[] {
  const configured = configuration.get<string>("cliPath")?.trim();
  const executableNames = platform === "win32"
    ? ["compass.exe"]
    : ["compass"];
  const pathCandidates = (environment.PATH ?? "")
    .split(path.delimiter)
    .filter(Boolean)
    .flatMap((directory) => executableNames.map((name) => ({
      executable: path.join(directory, name),
      source: "path" as const
    })));
  const commonDirectories = options.commonDirectories
    ?? commonInstallDirectories(environment, platform);
  const commonCandidates = commonDirectories
    .flatMap((directory) => executableNames.map((name) => ({
      executable: path.join(directory, name),
      source: "common" as const
    })));

  return deduplicateCandidates([
    ...(configured
      ? [{ executable: configured, source: "configured" as const }]
      : []),
    ...pathCandidates,
    ...commonCandidates
  ], platform, environment);
}

function commonInstallDirectories(
  environment: NodeJS.ProcessEnv,
  platform: NodeJS.Platform
): string[] {
  const home = environment.HOME ?? environment.USERPROFILE;
  if (platform === "win32") {
    return [
      ...(home
        ? [
          path.join(home, ".cargo", "bin"),
          path.join(home, ".local", "bin"),
          path.join(home, "scoop", "shims")
        ]
        : []),
      ...(environment.LOCALAPPDATA
        ? [path.join(environment.LOCALAPPDATA, "Programs", "Compass")]
        : []),
      ...(environment.ChocolateyInstall
        ? [path.join(environment.ChocolateyInstall, "bin")]
        : [])
    ];
  }
  return [
    ...(environment.COMPASS_INSTALL_DIR ? [environment.COMPASS_INSTALL_DIR] : []),
    ...(home
      ? [
        path.join(home, ".local", "bin"),
        path.join(home, ".cargo", "bin"),
        path.join(home, "bin")
      ]
      : []),
    ...(platform === "darwin" ? ["/opt/homebrew/bin"] : []),
    "/usr/local/bin",
    "/usr/bin"
  ];
}

function deduplicateCandidates(
  candidates: Candidate[],
  platform: NodeJS.Platform,
  environment: NodeJS.ProcessEnv
): Candidate[] {
  const seen = new Set<string>();
  const unique: Candidate[] = [];
  for (const candidate of candidates) {
    const resolved = resolveCompassPath(candidate.executable, environment);
    const key = platform === "win32" ? resolved.toLocaleLowerCase() : resolved;
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push({ ...candidate, executable: resolved });
  }
  return unique;
}

function compassVersion(executable: string): Promise<string | undefined> {
  return new Promise((resolve) => {
    execFile(
      executable,
      ["--version"],
      {
        encoding: "utf8",
        timeout: 2_000,
        windowsHide: true,
        maxBuffer: 16 * 1024
      },
      (error, stdout, stderr) => {
        if (error) {
          resolve(undefined);
          return;
        }
        resolve(parseCompassVersion(`${stdout}\n${stderr}`));
      }
    );
  });
}

function parseCompassVersion(output: string): string | undefined {
  const firstLine = output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean);
  if (!firstLine) return undefined;
  const semanticVersion = firstLine.match(
    /\bv?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)\b/
  );
  return semanticVersion?.[1] ?? firstLine;
}
