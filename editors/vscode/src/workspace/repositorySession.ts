import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import type { CapabilityReport } from "../cli/contracts";
import type { CompassProcessManager, RunningCommand } from "../cli/processManager";

export type GraphState = "available" | "not-materialized" | "building" | "failed";

export class RepositorySession {
  capabilities: CapabilityReport | undefined;
  capabilityError: string | undefined;
  graphState: GraphState = "not-materialized";
  activeWriter: RunningCommand | undefined;
  watch: RunningCommand | undefined;

  constructor(
    readonly id: string,
    readonly root: string,
    readonly processes: CompassProcessManager
  ) {}

  get graphPath(): string {
    return resolvePublishedArtifact(path.join(this.root, "compass-out"), "graph.json");
  }

  get programPath(): string {
    return resolvePublishedArtifact(path.join(this.root, "compass-out"), "program.json");
  }
}

export function resolvePublishedArtifact(outputDirectory: string, artifact: string): string {
  const pointer = path.join(outputDirectory, "current-snapshot");
  if (!existsSync(pointer)) {
    throw new Error(`Missing Compass current snapshot pointer: ${pointer}`);
  }
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
