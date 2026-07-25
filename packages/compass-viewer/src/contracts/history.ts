import { z } from "zod";
import { GraphViewModelSchema } from "./graph";

export const HistoryGraphStateSchema = z.enum([
  "graph_available",
  "not_materialized",
  "building",
  "failed"
]);
export const HistoryTimelineSchema = z.object({
  schema: z.literal("compass.history.timeline/1"),
  repositoryId: z.string(),
  selectedHead: z.string(),
  historyEnabled: z.boolean(),
  entries: z.array(z.object({
    commit: z.string(),
    parents: z.array(z.string()),
    authorName: z.string(),
    authorEmail: z.string(),
    authoredAtSeconds: z.number().int(),
    subject: z.string(),
    graphState: HistoryGraphStateSchema,
    presentationAvailable: z.boolean(),
    realization: z.string().nullable(),
    fingerprint: z.string().nullable(),
    job: z.unknown().nullable()
  }))
});
export const HistoricalGraphSchema = z.object({
  schema: z.literal("compass.history.viewer_graph/1"),
  commit: z.string(),
  realization: z.string(),
  fingerprint: z.string(),
  graph: GraphViewModelSchema
});
export const HistoryChangeCountsSchema = z.object({
  schema: z.literal("compass.history.change_counts/1"),
  commit: z.string(),
  parent: z.string(),
  counts: z.object({
    nodes: z.object({ added: z.number(), removed: z.number(), changed: z.number() }),
    edges: z.object({ added: z.number(), removed: z.number(), changed: z.number() }),
    hyperedges: z.object({ added: z.number(), removed: z.number(), changed: z.number() })
  })
});
export type HistoryTimeline = z.infer<typeof HistoryTimelineSchema>;
export type HistoryEntry = HistoryTimeline["entries"][number];
export type HistoricalGraph = z.infer<typeof HistoricalGraphSchema>;
export type HistoryChangeCounts = z.infer<typeof HistoryChangeCountsSchema>;
export type HistoryBuildState =
  | { status: "requesting" }
  | { status: "running" }
  | { status: "failed"; message: string };
export type HistoryOperationError = {
  operation: string;
  message: string;
};
