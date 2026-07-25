import path from "node:path";
import type { CapabilityReport } from "../cli/contracts";
import type { CompassProcessManager, RunningCommand } from "../cli/processManager";

export type GraphState = "available" | "not-materialized" | "building" | "failed";

export class RepositorySession {
  readonly graphPath: string;
  readonly programPath: string;
  capabilities: CapabilityReport | undefined;
  capabilityError: string | undefined;
  graphState: GraphState = "not-materialized";
  activeWriter: RunningCommand | undefined;
  watch: RunningCommand | undefined;

  constructor(
    readonly id: string,
    readonly root: string,
    readonly processes: CompassProcessManager
  ) {
    this.graphPath = path.join(root, "compass-out", "graph.json");
    this.programPath = path.join(root, "compass-out", "program.json");
  }
}
