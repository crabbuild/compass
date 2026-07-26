import {
  HistoryChangeCountsSchema,
  type HistoryChangeCounts
} from "@compass/viewer/contracts/history";
import type { RepositorySession } from "../workspace/repositorySession";

export async function loadChangeCounts(
  session: RepositorySession,
  commit: string,
  parent?: string
): Promise<HistoryChangeCounts> {
  const args = ["history", "change-counts", commit];
  if (parent) args.push("--parent", parent);
  args.push("--format", "json");
  return session.processes.runJson(
    session.root,
    args,
    HistoryChangeCountsSchema
  );
}
