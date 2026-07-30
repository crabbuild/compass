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
  const legacy = path.join(outputDirectory, artifact);
  const pointer = path.join(outputDirectory, ".compass-active-generation");
  if (!existsSync(pointer)) return legacy;
  const generation = readFileSync(pointer, "utf8").trim();
  if (!/^generation-[^/\\]+$/.test(generation)) {
    throw new Error(`Invalid Compass active generation pointer: ${pointer}`);
  }
  const active = path.join(outputDirectory, ".compass-generations", generation);
  if (!statSync(active).isDirectory()) {
    throw new Error(`Invalid Compass active generation directory: ${active}`);
  }
  if (existsSync(path.join(active, ".compass-build-incomplete"))) {
    throw new Error(`Incomplete Compass active generation: ${active}`);
  }
  return path.join(active, artifact);
}
