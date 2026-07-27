import { z } from "zod";

export const CallResolutionSchema = z.enum([
  "resolved",
  "inferred",
  "ambiguous",
  "unresolved"
]);
export const CallAnchorSchema = z.object({
  source_file: z.string(),
  start_byte: z.number().int().nonnegative(),
  end_byte: z.number().int().nonnegative()
});
export const CallNodeSchema = z.object({
  id: z.string(),
  symbol: z.string().nullable(),
  name: z.string(),
  file: z.string().nullable(),
  anchor: CallAnchorSchema.nullable().optional(),
  startLine: z.number().int().positive().nullable().optional(),
  endLine: z.number().int().positive().nullable().optional(),
  startByte: z.number().int().nonnegative().nullable().optional(),
  endByte: z.number().int().nonnegative().nullable().optional(),
  graphNodeId: z.string().nullable(),
  unresolved: z.boolean(),
  evidenceLayer: z.enum(["structural_graph", "program_ir", "combined"]).optional()
});
export const CallSiteSchema = z.union([
  z.object({
    anchor: CallAnchorSchema,
    evidence: z.array(z.string())
  }),
  z.object({
    sourceFile: z.string().nullable(),
    line: z.number().int().positive().nullable(),
    startByte: z.number().int().nonnegative().nullable(),
    endByte: z.number().int().nonnegative().nullable(),
    evidence: z.array(z.string())
  })
]);
export const CallEdgeSchema = z.object({
  id: z.string(),
  source: z.string(),
  target: z.string(),
  callee: z.string(),
  resolution: CallResolutionSchema,
  confidence: z.string().nullable().optional(),
  callSites: z.array(CallSiteSchema),
  evidenceLayer: z.enum(["structural_graph", "program_ir", "combined"]).optional()
});
export const CallGraphResponseSchema = z.object({
  schema: z.enum(["compass.program.call_graph/1", "compass.call_graph/1"]),
  rootSymbol: z.string(),
  direction: z.enum(["callers", "callees", "both"]),
  depth: z.number().int().positive(),
  nodes: z.array(CallNodeSchema),
  edges: z.array(CallEdgeSchema),
  truncated: z.boolean(),
  continuations: z.array(z.object({
    symbol: z.string(),
    direction: z.enum(["callers", "callees", "both"]),
    nextDepth: z.number().int().positive()
  })),
  coverage: z.object({
    resolved: z.number().int().nonnegative(),
    inferred: z.number().int().nonnegative(),
    ambiguous: z.number().int().nonnegative(),
    unresolved: z.number().int().nonnegative(),
    evidenceLayer: z.enum(["structural_graph", "program_ir", "combined"]).optional(),
    partial: z.boolean().optional(),
    limitations: z.array(z.string()).optional(),
    warning: z.string()
  })
});

export type CallGraphResponse = z.infer<typeof CallGraphResponseSchema>;
export type CallNode = z.infer<typeof CallNodeSchema>;
export type CallEdge = z.infer<typeof CallEdgeSchema>;
export type CallDirection = CallGraphResponse["direction"];
