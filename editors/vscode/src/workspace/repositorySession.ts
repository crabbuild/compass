import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import type { CapabilityReport } from "../cli/contracts";
import type { CompassProcessManager, RunningCommand } from "../cli/processManager";

export type GraphState = "available" | "not-materialized" | "building" | "failed";

export class RepositorySession {
  capabilities: CapabilityReport | undefined;
  capabilityError: string | undefined;
  graphError: string | undefined;
  graphState: GraphState = "not-materialized";
  activeWriter: RunningCommand | undefined;
  watch: RunningCommand | undefined;

  constructor(
    readonly id: string,
    readonly root: string,
    readonly processes: CompassProcessManager
  ) {}

  get graphPath(): string {
    const outputDirectory = path.join(this.root, "compass-out");
    return findPublishedArtifact(outputDirectory, "graph.json")
      ?? missingSnapshotError(outputDirectory);
  }

  get programPath(): string {
    const outputDirectory = path.join(this.root, "compass-out");
    return findPublishedArtifact(outputDirectory, "program.json")
      ?? missingSnapshotError(outputDirectory);
  }

  findGraphPath(): string | undefined {
    return findPublishedArtifact(path.join(this.root, "compass-out"), "graph.json");
  }
}

export function resolvePublishedArtifact(outputDirectory: string, artifact: string): string {
  return findPublishedArtifact(outputDirectory, artifact)
    ?? missingSnapshotError(outputDirectory);
}

export function findPublishedArtifact(
  outputDirectory: string,
  artifact: string
): string | undefined {
  const pointer = path.join(outputDirectory, "current-snapshot");
  if (!existsSync(pointer)) return undefined;
  const snapshot = readFileSync(pointer, "utf8").trim();
  if (!/^snapshot-[^/\\]+$/.test(snapshot)) {
    throw new Error(`Invalid Compass active snapshot pointer: ${pointer}`);
  }
  const active = path.join(outputDirectory, "snapshots", snapshot);
  if (!statSync(active).isDirectory()) {
    throw new Error(`Invalid Compass active snapshot directory: ${active}`);
  }
  if (existsSync(path.join(active, "build-incomplete"))) {
    throw new Error(`Incomplete Compass active snapshot: ${active}`);
  }
  return path.join(active, artifact);
}

function missingSnapshotError(outputDirectory: string): never {
  throw new Error(
    `Missing Compass current snapshot pointer: ${path.join(outputDirectory, "current-snapshot")}`
  );
}
