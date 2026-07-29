import { z } from "zod";

export const CallflowViewModelSchema = z.object({
  schema: z.literal("compass.viewer.callflow/2"),
  title: z.string(),
  sections: z.array(z.object({
    id: z.string(),
    name: z.string(),
    communities: z.array(z.string()),
    nodeCount: z.number().int().nonnegative(),
    internalCallCount: z.number().int().nonnegative(),
    nodes: z.array(z.object({
      id: z.string(),
      label: z.string(),
      kind: z.string(),
      sourceFile: z.string().nullable(),
      scope: z.enum(["production", "test", "generated", "vendor", "unknown"])
    })),
    edges: z.array(z.object({
      source: z.string(),
      target: z.string(),
      relation: z.string(),
      confidence: z.enum(["extracted", "inferred", "ambiguous"])
    }))
  })),
  overviewLinks: z.array(z.object({
    sourceSection: z.string(),
    targetSection: z.string(),
    calls: z.number().int().nonnegative()
  })),
  crossSectionCalls: z.array(z.object({
    source: z.string(),
    target: z.string(),
    sourceSection: z.string(),
    targetSection: z.string(),
    relation: z.string(),
    confidence: z.enum(["extracted", "inferred", "ambiguous"])
  })),
  coverage: z.object({
    internal: z.number().int().nonnegative(),
    crossSection: z.number().int().nonnegative(),
    unassigned: z.number().int().nonnegative()
  }),
  reportHighlights: z.array(z.string()),
  statistics: z.object({
    nodes: z.number().int().nonnegative(),
    edges: z.number().int().nonnegative(),
    communities: z.number().int().nonnegative(),
    hyperedges: z.number().int().nonnegative(),
    extracted: z.number().int().nonnegative(),
    inferred: z.number().int().nonnegative(),
    ambiguous: z.number().int().nonnegative()
  }),
  provenance: z.object({
    projectName: z.string(),
    builtAtCommit: z.string().nullable(),
    generatedAt: z.string().nullable()
  })
});

export type CallflowViewModel = z.infer<typeof CallflowViewModelSchema>;
