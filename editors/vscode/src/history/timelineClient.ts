import { HistoryTimelineSchema, type HistoryTimeline } from "@compass/viewer/contracts/history";
import type { RepositorySession } from "../workspace/repositorySession";

export async function loadTimeline(
  session: RepositorySession,
  page?: { limit: number; after?: string; revision?: string }
): Promise<HistoryTimeline> {
  return session.processes.runJson(
    session.root,
    [
      "history",
      "timeline",
      ...(page?.revision ? ["--rev", page.revision] : []),
      ...(page ? ["--limit", String(page.limit)] : []),
      ...(page?.after ? ["--after", page.after] : []),
      "--format",
      "json"
    ],
    HistoryTimelineSchema
  );
}
