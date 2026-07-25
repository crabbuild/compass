import { HistoryChangeCountsSchema, type HistoryChangeCounts } from "@compass/viewer";
import type { RepositorySession } from "../workspace/repositorySession";

export async function loadChangeCounts(
  session: RepositorySession,
  commit: string
): Promise<HistoryChangeCounts> {
  return session.processes.runJson(
    session.root,
    ["history", "change-counts", commit, "--format", "json"],
    HistoryChangeCountsSchema
  );
}
