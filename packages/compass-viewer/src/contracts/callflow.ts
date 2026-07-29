import { z } from "zod";

const ConfidenceSchema = z.enum(["extracted", "inferred", "ambiguous"]);
const SourceScopeSchema = z.enum([
  "production",
  "test",
  "generated",
  "vendor",
  "unknown"
]);

const RawCallflowViewModelSchema = z.object({
  schema: z.union([
    z.literal("compass.viewer.callflow/1"),
    z.literal("compass.viewer.callflow/2")
  ]),
  title: z.string(),
  sections: z.array(z.object({
    id: z.string(),
    name: z.string(),
    communities: z.array(z.string()),
    nodeCount: z.number().int().nonnegative().optional(),
    internalCallCount: z.number().int().nonnegative().optional(),
    nodes: z.array(z.object({
      id: z.string(),
      label: z.string(),
      kind: z.string(),
      sourceFile: z.string().nullable(),
      scope: SourceScopeSchema.optional()
    })),
    edges: z.array(z.object({
      source: z.string(),
      target: z.string(),
      relation: z.string(),
      confidence: ConfidenceSchema
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
    confidence: ConfidenceSchema
  })).optional(),
  coverage: z.object({
    internal: z.number().int().nonnegative(),
    crossSection: z.number().int().nonnegative(),
    unassigned: z.number().int().nonnegative()
  }).optional(),
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

export const CallflowViewModelSchema = RawCallflowViewModelSchema.transform((model) => {
  const legacyAggregateOnly = model.crossSectionCalls === undefined;
  const sections = model.sections.map((section) => ({
    ...section,
    nodeCount: section.nodeCount ?? section.nodes.length,
    internalCallCount: section.internalCallCount ?? section.edges.length,
    nodes: section.nodes.map((node) => ({
      ...node,
      // Original schema-v1 payloads predate source scopes. Treating their
      // contents as visible preserves the totals that payload advertised.
      scope: node.scope ?? ("production" as const)
    }))
  }));
  const internal = sections.reduce(
    (total, section) => total + section.internalCallCount,
    0
  );
  const crossSection = model.overviewLinks.reduce(
    (total, link) => total + link.calls,
    0
  );
  return {
    ...model,
    sections,
    crossSectionCalls: model.crossSectionCalls ?? [],
    coverage: model.coverage ?? {
      internal,
      crossSection,
      unassigned: Math.max(0, model.statistics.edges - internal - crossSection)
    },
    legacyAggregateOnly
  };
});

export type CallflowViewModel = z.infer<typeof CallflowViewModelSchema>;
