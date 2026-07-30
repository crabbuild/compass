import { z } from "zod";

export const ArchitectureScopeSchema = z.enum(["production", "all"]);
export const ArchitectureEvidenceSchema = z.enum([
  "all",
  "extracted",
  "inferred",
  "ambiguous"
]);
export const ArchitectureSourceScopeSchema = z.enum([
  "production",
  "test",
  "generated",
  "vendor",
  "unknown"
]);
export const ArchitectureConfidenceSchema = z.enum([
  "extracted",
  "inferred",
  "ambiguous"
]);

const ScopeCountsSchema = z.object({
  production: z.number().int().nonnegative(),
  test: z.number().int().nonnegative(),
  generated: z.number().int().nonnegative(),
  vendor: z.number().int().nonnegative(),
  unknown: z.number().int().nonnegative()
});

export const ArchitectureSectionSummarySchema = z.object({
  id: z.string(),
  name: z.string(),
  nodeCount: z.number().int().nonnegative(),
  totalNodeCount: z.number().int().nonnegative(),
  internalCallCount: z.number().int().nonnegative(),
  incomingCalls: z.number().int().nonnegative(),
  outgoingCalls: z.number().int().nonnegative(),
  scopes: ScopeCountsSchema
});

export const ArchitectureRouteSummarySchema = z.object({
  id: z.string(),
  sourceSection: z.string(),
  targetSection: z.string(),
  calls: z.number().int().nonnegative(),
  extracted: z.number().int().nonnegative(),
  inferred: z.number().int().nonnegative(),
  ambiguous: z.number().int().nonnegative()
});

export const ArchitectureOverviewSchema = z.object({
  title: z.string(),
  scope: ArchitectureScopeSchema,
  evidence: ArchitectureEvidenceSchema,
  sections: z.array(ArchitectureSectionSummarySchema),
  routes: z.array(ArchitectureRouteSummarySchema),
  statistics: z.object({
    visibleNodes: z.number().int().nonnegative(),
    totalNodes: z.number().int().nonnegative(),
    visibleCalls: z.number().int().nonnegative(),
    totalCalls: z.number().int().nonnegative(),
    communities: z.number().int().nonnegative(),
    extracted: z.number().int().nonnegative(),
    inferred: z.number().int().nonnegative(),
    ambiguous: z.number().int().nonnegative()
  }),
  coverage: z.object({
    internal: z.number().int().nonnegative(),
    crossSection: z.number().int().nonnegative(),
    unassigned: z.number().int().nonnegative()
  }),
  provenance: z.object({
    projectName: z.string(),
    builtAtCommit: z.string().nullable(),
    generatedAt: z.string().nullable()
  })
});

export const ArchitectureSymbolSchema = z.object({
  id: z.string(),
  label: z.string(),
  kind: z.string(),
  sourceFile: z.string().nullable(),
  scope: ArchitectureSourceScopeSchema,
  sectionId: z.string()
});

export const ArchitectureCallSchema = z.object({
  id: z.string(),
  source: z.string(),
  target: z.string(),
  sourceLabel: z.string(),
  targetLabel: z.string(),
  sourceFile: z.string().nullable(),
  targetFile: z.string().nullable(),
  sourceSection: z.string(),
  targetSection: z.string(),
  relation: z.string(),
  confidence: ArchitectureConfidenceSchema
});

const PageSchema = z.object({
  page: z.number().int().positive(),
  pageSize: z.number().int().min(1).max(100),
  pageCount: z.number().int().positive(),
  total: z.number().int().nonnegative(),
  start: z.number().int().nonnegative(),
  end: z.number().int().nonnegative()
});

export const ArchitectureSectionPageSchema = z.discriminatedUnion("kind", [
  PageSchema.extend({
    kind: z.literal("symbols"),
    sectionId: z.string(),
    items: z.array(ArchitectureSymbolSchema)
  }),
  PageSchema.extend({
    kind: z.literal("calls"),
    sectionId: z.string(),
    items: z.array(ArchitectureCallSchema)
  })
]);

export const ArchitectureRoutePageSchema = PageSchema.extend({
  routeId: z.string(),
  sourceSection: z.string(),
  targetSection: z.string(),
  items: z.array(ArchitectureCallSchema)
});

export const ArchitectureSearchResultSchema = z.object({
  id: z.string(),
  kind: z.enum(["section", "symbol", "call"]),
  label: z.string(),
  detail: z.string(),
  sectionId: z.string().nullable(),
  routeId: z.string().nullable(),
  sourceFile: z.string().nullable()
});

export const ArchitectureSearchPageSchema = PageSchema.extend({
  query: z.string(),
  items: z.array(ArchitectureSearchResultSchema)
});

export type ArchitectureScope = z.infer<typeof ArchitectureScopeSchema>;
export type ArchitectureEvidence = z.infer<typeof ArchitectureEvidenceSchema>;
export type ArchitectureSourceScope = z.infer<typeof ArchitectureSourceScopeSchema>;
export type ArchitectureSectionSummary = z.infer<typeof ArchitectureSectionSummarySchema>;
export type ArchitectureRouteSummary = z.infer<typeof ArchitectureRouteSummarySchema>;
export type ArchitectureOverview = z.infer<typeof ArchitectureOverviewSchema>;
export type ArchitectureSymbol = z.infer<typeof ArchitectureSymbolSchema>;
export type ArchitectureCall = z.infer<typeof ArchitectureCallSchema>;
export type ArchitectureSectionPage = z.infer<typeof ArchitectureSectionPageSchema>;
export type ArchitectureRoutePage = z.infer<typeof ArchitectureRoutePageSchema>;
export type ArchitectureSearchResult = z.infer<typeof ArchitectureSearchResultSchema>;
export type ArchitectureSearchPage = z.infer<typeof ArchitectureSearchPageSchema>;
