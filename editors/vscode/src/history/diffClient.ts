import {
  SemanticDiffReportSchema,
  type SemanticDiffReport
} from "@compass/viewer/contracts/history";
import type { RepositorySession } from "../workspace/repositorySession";

export async function loadSemanticDiff(
  session: RepositorySession,
  parent: string,
  commit: string,
  signal?: AbortSignal
): Promise<SemanticDiffReport> {
  return session.processes.runJson(
    session.root,
    ["diff", parent, commit, "--format", "json"],
    SemanticDiffReportSchema,
    signal
  );
}
