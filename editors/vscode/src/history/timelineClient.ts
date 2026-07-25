import { HistoryTimelineSchema, type HistoryTimeline } from "@compass/viewer/contracts/history";
import type { RepositorySession } from "../workspace/repositorySession";

export async function loadTimeline(session: RepositorySession): Promise<HistoryTimeline> {
  return session.processes.runJson(
    session.root,
    ["history", "timeline", "--format", "json"],
    HistoryTimelineSchema
  );
}
