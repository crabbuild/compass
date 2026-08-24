import { z } from "zod";
import { CodeEdgeDetailsSchema, CodeEvidenceSchema } from "./codeQuery";

export const GRAPH_VIEWER_SCHEMA = "compass.viewer.graph/1" as const;

export const SourceLocationSchema = z.object({
  file: z.string(),
  startLine: z.number().int().positive().optional(),
  endLine: z.number().int().positive().optional(),
  startByte: z.number().int().nonnegative().optional(),
  endByte: z.number().int().nonnegative().optional()
}).passthrough();

export const GraphFieldChangeSchema = z.object({
  field: z.string().min(1),
  before: z.unknown().optional(),
  after: z.unknown().optional()
});

export const GraphRecordEvidenceSchema = z.object({
  before: z.record(z.string(), z.unknown()).optional(),
  after: z.record(z.string(), z.unknown()).optional(),
  fields: z.array(GraphFieldChangeSchema)
});

export const AgentChallengeSchema = z.object({
  challenge: z.string().startsWith("challenge:"),
  targetId: z.string().min(1),
  effect: z.enum(["flag", "mask"]),
  masked: z.boolean(),
  certificateDigest: z.string().regex(/^[0-9a-f]{64}$/),
  summary: z.string()
}).strict();

export const AgentRetractionsSchema = z.object({
  total: z.number().int().nonnegative(),
  examples: z.array(z.object({
    kind: z.enum(["assertion", "challenge"]),
    id: z.string().min(1),
    reasonCode: z.string().min(1),
    explanation: z.string(),
    sequence: z.number().int().nonnegative()
  }).strict()),
  omittedExamples: z.number().int().nonnegative()
}).strict();

export const DocumentOcrProfileSchema = z.record(z.string(), z.unknown());

export const DocumentOriginSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("native") }).passthrough(),
  z.object({
    kind: z.literal("ocr"),
    profile: DocumentOcrProfileSchema,
    confidence_bps: z.number().int().min(0).max(10_000)
  }).passthrough()
]);

export const DocumentLocatorSchema = z.object({
  kind: z.string().min(1)
}).passthrough();

export const GraphDocumentSchema = z.object({
  role: z.enum(["root", "block"]).optional(),
  kind: z.string().optional(),
  format: z.string().optional(),
  text: z.string().optional(),
  ordinal: z.number().int().nonnegative().optional(),
  complete: z.boolean().optional(),
  visualCoverage: z.enum(["not_requested", "complete", "partial", "failed"]).optional(),
  ocrMode: z.enum(["off", "auto", "always"]).optional(),
  origin: DocumentOriginSchema.optional(),
  locator: DocumentLocatorSchema.optional(),
  ocrProfile: DocumentOcrProfileSchema.optional()
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
  detailAvailable: z.boolean().optional(),
  learningStatus: z.string().optional(),
  learningStale: z.boolean().optional(),
  depth: z.number().int().nonnegative().optional(),
  root: z.boolean().optional(),
  agentAssertion: z.string().startsWith("assertion:").optional(),
  agentSummary: z.string().optional(),
  groundingStatus: z.literal("GROUNDED").optional(),
  challenged: z.boolean().optional(),
  challenge: AgentChallengeSchema.optional(),
  change: z.enum(["added", "removed", "changed", "unchanged"]).optional(),
  evidence: GraphRecordEvidenceSchema.optional(),
  codeEvidence: z.array(CodeEvidenceSchema).optional(),
  source: SourceLocationSchema.optional(),
  color: z.object({
    background: z.string(),
    border: z.string()
  }).passthrough().optional(),
  document: GraphDocumentSchema.optional()
}).passthrough();

export const GraphEdgeSchema = z.object({
  id: z.string().min(1),
  source: z.string().min(1),
  target: z.string().min(1),
  relation: z.string(),
  weight: z.number().int().positive().optional(),
  change: z.enum(["added", "removed", "changed", "unchanged"]).optional(),
  evidence: GraphRecordEvidenceSchema.optional(),
  codeEvidence: z.array(CodeEvidenceSchema).optional(),
  details: CodeEdgeDetailsSchema.nullable().optional(),
  relationshipSite: SourceLocationSchema.optional(),
  confidence: z.enum([
    "extracted",
    "inferred",
    "ambiguous",
    "aggregated"
  ]).optional(),
  agentAssertion: z.string().startsWith("assertion:").optional(),
  agentSummary: z.string().optional(),
  groundingStatus: z.literal("GROUNDED").optional(),
  challenged: z.boolean().optional(),
  challenge: AgentChallengeSchema.optional()
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
  hyperedges: z.array(z.unknown()).default([]),
  effectiveGraph: z.object({
    effectiveIdentity: z.string().regex(/^[0-9a-f]{64}$/),
    baseGeneration: z.object({
      generationId: z.string().min(1),
      graphDigest: z.string().regex(/^[0-9a-f]{64}$/)
    }).strict(),
    overlayRevision: z.string().regex(/^[0-9a-f]{64}$/),
    compositionProfile: z.enum(["augment", "curated"]),
    retractions: AgentRetractionsSchema,
    omissions: z.object({
      total: z.number().int().nonnegative(),
      direct: z.number().int().nonnegative(),
      cascaded: z.number().int().nonnegative(),
      examples: z.array(z.unknown()),
      omittedExamples: z.number().int().nonnegative()
    }).strict()
  }).strict().optional()
}).passthrough();

export type SourceLocation = z.infer<typeof SourceLocationSchema>;
export type GraphFieldChange = z.infer<typeof GraphFieldChangeSchema>;
export type GraphRecordEvidence = z.infer<typeof GraphRecordEvidenceSchema>;
export type DocumentOcrProfile = z.infer<typeof DocumentOcrProfileSchema>;
export type DocumentOrigin = z.infer<typeof DocumentOriginSchema>;
export type DocumentLocator = z.infer<typeof DocumentLocatorSchema>;
export type GraphDocument = z.infer<typeof GraphDocumentSchema>;
export type GraphNode = z.infer<typeof GraphNodeSchema>;
export type GraphEdge = z.infer<typeof GraphEdgeSchema>;
export type GraphViewModel = z.infer<typeof GraphViewModelSchema>;
