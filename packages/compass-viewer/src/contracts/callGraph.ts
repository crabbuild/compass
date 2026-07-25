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
  anchor: CallAnchorSchema.nullable(),
  graphNodeId: z.string().nullable(),
  unresolved: z.boolean()
});
export const CallEdgeSchema = z.object({
  id: z.string(),
  source: z.string(),
  target: z.string(),
  callee: z.string(),
  resolution: CallResolutionSchema,
  callSites: z.array(z.object({
    anchor: CallAnchorSchema,
    evidence: z.array(z.string())
  }))
});
export const CallGraphResponseSchema = z.object({
  schema: z.literal("compass.program.call_graph/1"),
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
    warning: z.string()
  })
});

export type CallGraphResponse = z.infer<typeof CallGraphResponseSchema>;
export type CallNode = z.infer<typeof CallNodeSchema>;
export type CallEdge = z.infer<typeof CallEdgeSchema>;
export type CallDirection = CallGraphResponse["direction"];
