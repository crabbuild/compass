import { z } from "zod";

export const GRAPH_VIEWER_SCHEMA = "compass.viewer.graph/1" as const;

export const SourceLocationSchema = z.object({
  file: z.string(),
  startLine: z.number().int().positive().optional(),
  endLine: z.number().int().positive().optional(),
  startByte: z.number().int().nonnegative().optional(),
  endByte: z.number().int().nonnegative().optional()
}).passthrough();

export const GraphNodeSchema = z.object({
  id: z.string().min(1),
  label: z.string(),
  kind: z.string().optional(),
  community: z.number().int(),
  communityName: z.string().optional(),
  degree: z.number().int().nonnegative().optional(),
  language: z.string().optional(),
  signature: z.string().optional(),
  size: z.number().positive().optional(),
  memberCount: z.number().int().nonnegative().optional(),
  learningStatus: z.string().optional(),
  learningStale: z.boolean().optional(),
  source: SourceLocationSchema.optional(),
  color: z.object({
    background: z.string(),
    border: z.string()
  }).passthrough().optional()
}).passthrough();

export const GraphEdgeSchema = z.object({
  id: z.string().min(1),
  source: z.string().min(1),
  target: z.string().min(1),
  relation: z.string(),
  confidence: z.enum(["extracted", "inferred", "ambiguous"]).optional()
}).passthrough();

export const CommunitySchema = z.object({
  id: z.number().int(),
  label: z.string(),
  color: z.string(),
  hidden: z.boolean().default(false)
}).passthrough();

export const GraphViewModelSchema = z.object({
  schema: z.literal(GRAPH_VIEWER_SCHEMA),
  title: z.string(),
  stats: z.object({
    nodes: z.number().int().nonnegative(),
    edges: z.number().int().nonnegative(),
    communities: z.number().int().nonnegative(),
    aggregated: z.boolean()
  }).passthrough(),
  nodes: z.array(GraphNodeSchema),
  edges: z.array(GraphEdgeSchema),
  communities: z.array(CommunitySchema),
  hyperedges: z.array(z.unknown()).default([])
}).passthrough();

export type SourceLocation = z.infer<typeof SourceLocationSchema>;
export type GraphNode = z.infer<typeof GraphNodeSchema>;
export type GraphEdge = z.infer<typeof GraphEdgeSchema>;
export type GraphViewModel = z.infer<typeof GraphViewModelSchema>;
