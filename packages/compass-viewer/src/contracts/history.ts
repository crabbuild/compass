import { z } from "zod";
import { GraphViewModelSchema } from "./graph";

const MAX_SEMANTIC_FINDINGS = 5_000;
const MAX_SEMANTIC_EVIDENCE = 20;
const MAX_GRAPH_DELTA_NODES = 10_000;
const MAX_GRAPH_DELTA_EDGES = 200_000;

const CompletenessSchema = z.enum(["complete", "partial", "unavailable"]);
const SemanticEvidenceRefSchema = z.object({
  source_file: z.string(),
  start_byte: z.number().int().nonnegative().optional(),
  end_byte: z.number().int().nonnegative().optional(),
  record_key: z.string().optional(),
  capability: z.string()
});
const SemanticNodeDeltaSchema = z.object({
  id: z.string().min(1),
  label: z.string(),
  kind: z.string(),
  source_file: z.string(),
  changed_fields: z.array(z.string())
});
const SemanticEdgeDeltaSchema = z.object({
  source: z.string().min(1),
  target: z.string().min(1),
  relation: z.string(),
  key: z.string(),
  source_file: z.string(),
  changed_fields: z.array(z.string())
});
const SourceFileDeltaSchema = z.object({
  old_path: z.string().nullable(),
  new_path: z.string().nullable(),
  status: z.enum(["added", "modified", "deleted", "renamed"]),
  hunks: z.array(z.object({
    old_start: z.number().int().nonnegative(),
    old_lines: z.number().int().nonnegative(),
    new_start: z.number().int().nonnegative(),
    new_lines: z.number().int().nonnegative()
  })),
  patch: z.string()
});

export const SemanticDiffReportSchema = z.object({
  schema: z.literal("compass.semantic_diff.report/1"),
  comparison: z.object({
    old_commit: z.string().min(1),
    new_commit: z.string().min(1),
    fingerprint: z.string().min(1)
  }),
  findings: z.array(z.object({
    id: z.string().min(1),
    finding_type: z.enum([
      "contract_change",
      "behavior_change",
      "dependency_change",
      "impact_change",
      "verification_gap",
      "structural_change"
    ]),
    subject: z.string().min(1),
    origin: z.enum(["direct", "derived"]),
    headline: z.string(),
    explanation: z.string(),
    compatibility: z.enum([
      "proven_break",
      "possible_break",
      "compatible",
      "behavioral",
      "not_applicable",
      "indeterminate"
    ]),
    confidence: z.enum(["exact", "probable", "inferred", "unknown"]),
    review_priority: z.number().int().nonnegative(),
    public_surface: z.boolean(),
    routine: z.boolean(),
    before: z.unknown().optional(),
    after: z.unknown().optional(),
    affected_consumers: z.array(z.object({
      symbol_id: z.string(),
      display_name: z.string(),
      source_file: z.string(),
      distance: z.number().int().nonnegative()
    })),
    witness_paths: z.array(z.object({
      consumer: z.string(),
      confidence: z.enum(["exact", "probable", "inferred", "unknown"]),
      hops: z.array(z.object({
        source: z.string(),
        relation: z.string(),
        target: z.string(),
        confidence: z.enum(["exact", "probable", "inferred", "unknown"])
      }))
    })),
    verification: z.object({
      state: z.enum(["unknown", "covered", "gap", "partial", "stale", "failing", "not_run"]),
      exact_tests: z.array(z.string()),
      recommended_tests: z.array(z.string()),
      reason: z.string()
    }),
    reviewer_action: z.string(),
    evidence: z.array(SemanticEvidenceRefSchema).max(MAX_SEMANTIC_EVIDENCE),
    completeness: z.record(z.string(), CompletenessSchema)
  })).max(MAX_SEMANTIC_FINDINGS),
  feature_groups: z.array(z.object({
    id: z.string(),
    headline: z.string(),
    summary: z.string(),
    finding_ids: z.array(z.string()),
    source_files: z.array(z.string()),
    public_surface_changes: z.number().int().nonnegative(),
    behavior_changes: z.number().int().nonnegative(),
    dependency_changes: z.number().int().nonnegative(),
    test_changes: z.number().int().nonnegative()
  })),
  collapsed_groups: z.array(z.object({
    label: z.string(),
    count: z.number().int().nonnegative(),
    finding_ids: z.array(z.string())
  })),
  source_changes: z.array(SourceFileDeltaSchema),
  graph_delta: z.object({
    added_nodes: z.array(SemanticNodeDeltaSchema).max(MAX_GRAPH_DELTA_NODES),
    removed_nodes: z.array(SemanticNodeDeltaSchema).max(MAX_GRAPH_DELTA_NODES),
    changed_nodes: z.array(SemanticNodeDeltaSchema).max(MAX_GRAPH_DELTA_NODES),
    added_edges: z.array(SemanticEdgeDeltaSchema).max(MAX_GRAPH_DELTA_EDGES),
    removed_edges: z.array(SemanticEdgeDeltaSchema).max(MAX_GRAPH_DELTA_EDGES),
    changed_edges: z.array(SemanticEdgeDeltaSchema).max(MAX_GRAPH_DELTA_EDGES),
    collapsed_attribute_changes: z.record(z.string(), z.number().int().nonnegative())
  }),
  entity_display_names: z.record(z.string(), z.string()).optional(),
  completeness: z.record(z.string(), CompletenessSchema),
  limitations: z.array(z.string())
});

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
  totalEntries: z.number().int().nonnegative().nullable().optional(),
  hasMore: z.boolean().optional(),
  nextCursor: z.string().nullable().optional(),
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
export type SemanticDiffReport = z.infer<typeof SemanticDiffReportSchema>;
export type HistoryBuildState =
  | { status: "requesting" }
  | { status: "running" }
  | { status: "failed"; message: string };
export type HistoryOperationError = {
  operation: string;
  message: string;
};
