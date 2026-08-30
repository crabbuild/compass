import { z } from "zod";

export const ARCHITECTURE_VIEWER_SCHEMA = "compass.viewer.architecture/1" as const;

export const RawArchitectureScopeSchema = z.enum(["production", "all_code"]);
export const ArchitectureRelationClassSchema = z.enum([
  "execution", "dependency", "type", "structure", "contextual", "unknown"
]);
export const ArchitectureLensSchema = z.enum([
  "architecture", "execution", "dependency", "type", "structure", "all"
]);
export const ArchitectureGroupKindSchema = z.enum(["owner", "subsystem"]);
export const ArchitectureRouteLevelSchema = z.enum(["overview", "detail"]);
export const ArchitectureNameProvenanceSchema = z.enum([
  "overlay", "persisted", "owner", "path", "declaration", "hub", "provider", "fallback"
]);
export const ArchitectureQualityStatusSchema = z.enum(["good", "degraded", "insufficient"]);
export const ArchitectureDiagnosticSeveritySchema = z.enum(["info", "warning", "error"]);

const RawSourceScopeSchema = z.enum([
  "production", "test", "generated", "vendor", "documentation", "unknown"
]);
const RawSourceCountsSchema = z.strictObject({
  production: z.number().int().nonnegative(),
  test: z.number().int().nonnegative(),
  generated: z.number().int().nonnegative(),
  vendor: z.number().int().nonnegative(),
  documentation: z.number().int().nonnegative(),
  unknown: z.number().int().nonnegative()
});
const RelationClassCountsSchema = z.strictObject({
  execution: z.number().int().nonnegative(),
  dependency: z.number().int().nonnegative(),
  type: z.number().int().nonnegative(),
  structure: z.number().int().nonnegative(),
  contextual: z.number().int().nonnegative(),
  unknown: z.number().int().nonnegative()
});
const EvidenceCountsSchema = z.strictObject({
  extracted: z.number().int().nonnegative(),
  inferred: z.number().int().nonnegative(),
  ambiguous: z.number().int().nonnegative()
});
const ArchitectureNodeSchema = z.strictObject({
  id: z.string().min(1),
  label: z.string(),
  kind: z.string(),
  sourceFile: z.string().nullable(),
  sourceScope: RawSourceScopeSchema,
  scopeReason: z.string().min(1),
  community: z.number().int().nonnegative().nullable()
});
const ArchitectureRelationshipSchema = z.strictObject({
  id: z.string().min(1),
  source: z.string().min(1),
  target: z.string().min(1),
  relation: z.string(),
  relationClass: ArchitectureRelationClassSchema,
  confidence: z.enum(["extracted", "inferred", "ambiguous"])
});
const ArchitectureGroupNameSchema = z.strictObject({
  value: z.string().min(1),
  provenance: ArchitectureNameProvenanceSchema,
  membershipSignature: z.string().min(1),
  quality: z.number().int().min(0).max(100),
  evidence: z.array(z.string())
});
const ArchitectureGroupSchema = z.strictObject({
  id: z.string().min(1),
  parentId: z.string().nullable(),
  kind: ArchitectureGroupKindSchema,
  rank: z.number().int().positive(),
  name: ArchitectureGroupNameSchema,
  ownerKey: z.string().min(1),
  communityIds: z.array(z.number().int().nonnegative()),
  nodeCount: z.number().int().nonnegative(),
  relationshipCount: z.number().int().nonnegative(),
  neighborCount: z.number().int().nonnegative(),
  cohesion: z.number().finite(),
  sourceScopes: RawSourceCountsSchema,
  pinned: z.boolean()
});
const ArchitectureMembershipSchema = z.strictObject({
  nodeIndex: z.number().int().nonnegative(),
  groupIndex: z.number().int().nonnegative()
});
const ArchitectureRouteSchema = z.strictObject({
  id: z.string().min(1),
  level: ArchitectureRouteLevelSchema,
  ownerId: z.string().nullable(),
  sourceGroup: z.string().min(1),
  targetGroup: z.string().min(1),
  relationshipCount: z.number().int().nonnegative(),
  relationClasses: RelationClassCountsSchema,
  evidence: EvidenceCountsSchema
});
const ArchitectureCoverageSchema = z.strictObject({
  admitted: z.number().int().nonnegative(),
  internal: z.number().int().nonnegative(),
  crossGroup: z.number().int().nonnegative(),
  unassigned: z.number().int().nonnegative(),
  relationClasses: RelationClassCountsSchema
});
const ArchitectureOmissionsSchema = z.strictObject({
  totalGroups: z.number().int().nonnegative(),
  shownGroups: z.number().int().nonnegative(),
  omittedGroups: z.number().int().nonnegative(),
  representedNodes: z.number().int().nonnegative(),
  omittedNodes: z.number().int().nonnegative(),
  representedRelationships: z.number().int().nonnegative(),
  omittedRelationships: z.number().int().nonnegative(),
  witnessGroupIds: z.array(z.string()),
  maxOverviewGroups: z.number().int().positive(),
  maxOverviewRoutes: z.number().int().positive()
});
const ArchitectureQualityDiagnosticSchema = z.strictObject({
  code: z.string().min(1),
  severity: ArchitectureDiagnosticSeveritySchema,
  message: z.string(),
  observed: z.number().nullable(),
  threshold: z.number().nullable(),
  witnesses: z.array(z.string()),
  recommendedAction: z.string()
});
const ArchitectureQualitySchema = z.strictObject({
  status: ArchitectureQualityStatusSchema,
  metrics: z.strictObject({
    sourceScopes: RawSourceCountsSchema,
    unknownSourceFraction: z.number().min(0).max(1),
    generatedVendorLeakage: z.number().int().nonnegative(),
    representedNodeFraction: z.number().min(0).max(1),
    representedRelationshipFraction: z.number().min(0).max(1),
    duplicateNames: z.number().int().nonnegative(),
    fallbackNames: z.number().int().nonnegative(),
    largestGroupFraction: z.number().min(0).max(1),
    unknownRelations: z.number().int().nonnegative(),
    unassignedNodes: z.number().int().nonnegative(),
    unassignedRelationships: z.number().int().nonnegative()
  }),
  diagnostics: z.array(ArchitectureQualityDiagnosticSchema)
});
const ScopeProjectionSchema = z.strictObject({
  scope: RawArchitectureScopeSchema,
  defaultLens: ArchitectureLensSchema,
  groups: z.array(ArchitectureGroupSchema),
  memberships: z.array(ArchitectureMembershipSchema),
  routes: z.array(ArchitectureRouteSchema),
  overviewGroupIds: z.array(z.string()),
  overviewRouteIds: z.array(z.string()),
  coverage: ArchitectureCoverageSchema,
  omissions: ArchitectureOmissionsSchema,
  quality: ArchitectureQualitySchema
});
const ProjectionLimitsSchema = z.strictObject({
  maxNodes: z.number().int().positive(),
  maxRelationships: z.number().int().positive(),
  maxGroups: z.number().int().positive(),
  maxRoutes: z.number().int().positive(),
  maxOverviewGroups: z.number().int().positive(),
  maxOverviewRoutes: z.number().int().positive(),
  maxNameCandidates: z.number().int().positive(),
  maxNameEvidence: z.number().int().positive(),
  maxDiagnostics: z.number().int().positive(),
  maxOmissionWitnesses: z.number().int().positive()
});

export const ArchitectureViewModelSchema = z.strictObject({
  schema: z.literal(ARCHITECTURE_VIEWER_SCHEMA),
  title: z.string(),
  nodes: z.array(ArchitectureNodeSchema),
  relationships: z.array(ArchitectureRelationshipSchema),
  projections: z.array(ScopeProjectionSchema).min(1),
  statistics: z.strictObject({
    nodes: z.number().int().nonnegative(),
    relationships: z.number().int().nonnegative(),
    communities: z.number().int().nonnegative(),
    extracted: z.number().int().nonnegative(),
    inferred: z.number().int().nonnegative(),
    ambiguous: z.number().int().nonnegative()
  }),
  provenance: z.strictObject({
    projectName: z.string(),
    builtAtCommit: z.string().nullable(),
    generatedAt: z.string().nullable()
  }),
  limits: ProjectionLimitsSchema
}).superRefine((model, context) => {
  const nodeIds = new Set(model.nodes.map((node) => node.id));
  if (nodeIds.size !== model.nodes.length) {
    context.addIssue({ code: "custom", path: ["nodes"], message: "node IDs must be unique" });
  }
  for (const [index, relationship] of model.relationships.entries()) {
    if (!nodeIds.has(relationship.source) || !nodeIds.has(relationship.target)) {
      context.addIssue({
        code: "custom",
        path: ["relationships", index],
        message: "relationship endpoints must identify included nodes"
      });
    }
  }
  for (const [projectionIndex, projection] of model.projections.entries()) {
    const groupIds = new Set(projection.groups.map((group) => group.id));
    for (const [index, membership] of projection.memberships.entries()) {
      if (membership.nodeIndex >= model.nodes.length
        || membership.groupIndex >= projection.groups.length) {
        context.addIssue({
          code: "custom",
          path: ["projections", projectionIndex, "memberships", index],
          message: "membership indexes must identify an included node and group"
        });
      }
    }
    for (const [index, route] of projection.routes.entries()) {
      if (!groupIds.has(route.sourceGroup) || !groupIds.has(route.targetGroup)) {
        context.addIssue({
          code: "custom",
          path: ["projections", projectionIndex, "routes", index],
          message: "route endpoints must identify included groups"
        });
      }
    }
  }
});

export type ArchitectureViewModel = z.infer<typeof ArchitectureViewModelSchema>;
export type ArchitectureRawNode = z.infer<typeof ArchitectureNodeSchema>;
export type ArchitectureRawRelationship = z.infer<typeof ArchitectureRelationshipSchema>;
export type ArchitectureRawGroup = z.infer<typeof ArchitectureGroupSchema>;
export type ArchitectureRawProjection = z.infer<typeof ScopeProjectionSchema>;

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
  "documentation",
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
  documentation: z.number().int().nonnegative(),
  unknown: z.number().int().nonnegative()
});

export const ArchitectureGroupSummarySchema = z.object({
  id: z.string(),
  name: z.string(),
  nodeCount: z.number().int().nonnegative(),
  totalNodeCount: z.number().int().nonnegative(),
  internalRelationshipCount: z.number().int().nonnegative(),
  incomingRelationships: z.number().int().nonnegative(),
  outgoingRelationships: z.number().int().nonnegative(),
  scopes: ScopeCountsSchema
});

export const ArchitectureRouteSummarySchema = z.object({
  id: z.string(),
  sourceGroup: z.string(),
  targetGroup: z.string(),
  relationships: z.number().int().nonnegative(),
  extracted: z.number().int().nonnegative(),
  inferred: z.number().int().nonnegative(),
  ambiguous: z.number().int().nonnegative()
});

export const ArchitectureOverviewSchema = z.object({
  title: z.string(),
  scope: ArchitectureScopeSchema,
  evidence: ArchitectureEvidenceSchema,
  lens: ArchitectureLensSchema,
  groups: z.array(ArchitectureGroupSummarySchema),
  routes: z.array(ArchitectureRouteSummarySchema),
  statistics: z.object({
    visibleNodes: z.number().int().nonnegative(),
    totalNodes: z.number().int().nonnegative(),
    visibleRelationships: z.number().int().nonnegative(),
    totalRelationships: z.number().int().nonnegative(),
    communities: z.number().int().nonnegative(),
    extracted: z.number().int().nonnegative(),
    inferred: z.number().int().nonnegative(),
    ambiguous: z.number().int().nonnegative()
  }),
  coverage: z.object({
    internal: z.number().int().nonnegative(),
    crossGroup: z.number().int().nonnegative(),
    unassigned: z.number().int().nonnegative()
  }),
  omissions: ArchitectureOmissionsSchema,
  quality: ArchitectureQualitySchema,
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
  groupId: z.string()
});

export const ArchitectureRelationshipRecordSchema = z.object({
  id: z.string(),
  source: z.string(),
  target: z.string(),
  sourceLabel: z.string(),
  targetLabel: z.string(),
  sourceFile: z.string().nullable(),
  targetFile: z.string().nullable(),
  sourceGroup: z.string(),
  targetGroup: z.string(),
  relation: z.string(),
  relationClass: ArchitectureRelationClassSchema,
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

export const ArchitectureGroupPageSchema = z.discriminatedUnion("kind", [
  PageSchema.extend({
    kind: z.literal("symbols"),
    groupId: z.string(),
    items: z.array(ArchitectureSymbolSchema)
  }),
  PageSchema.extend({
    kind: z.literal("relationships"),
    groupId: z.string(),
    items: z.array(ArchitectureRelationshipRecordSchema)
  })
]);

export const ArchitectureRoutePageSchema = PageSchema.extend({
  routeId: z.string(),
  sourceGroup: z.string(),
  targetGroup: z.string(),
  items: z.array(ArchitectureRelationshipRecordSchema)
});

export const ArchitectureSearchResultSchema = z.object({
  id: z.string(),
  kind: z.enum(["group", "symbol", "relationship"]),
  label: z.string(),
  detail: z.string(),
  groupId: z.string().nullable(),
  routeId: z.string().nullable(),
  sourceFile: z.string().nullable()
});

export const ArchitectureSearchPageSchema = PageSchema.extend({
  query: z.string(),
  items: z.array(ArchitectureSearchResultSchema)
});

export type ArchitectureScope = z.infer<typeof ArchitectureScopeSchema>;
export type ArchitectureEvidence = z.infer<typeof ArchitectureEvidenceSchema>;
export type ArchitectureLens = z.infer<typeof ArchitectureLensSchema>;
export type ArchitectureSourceScope = z.infer<typeof ArchitectureSourceScopeSchema>;
export type ArchitectureGroupSummary = z.infer<typeof ArchitectureGroupSummarySchema>;
export type ArchitectureRouteSummary = z.infer<typeof ArchitectureRouteSummarySchema>;
export type ArchitectureOverview = z.infer<typeof ArchitectureOverviewSchema>;
export type ArchitectureSymbol = z.infer<typeof ArchitectureSymbolSchema>;
export type ArchitectureRelationshipRecord = z.infer<typeof ArchitectureRelationshipRecordSchema>;
export type ArchitectureGroupPage = z.infer<typeof ArchitectureGroupPageSchema>;
export type ArchitectureRoutePage = z.infer<typeof ArchitectureRoutePageSchema>;
export type ArchitectureSearchResult = z.infer<typeof ArchitectureSearchResultSchema>;
export type ArchitectureSearchPage = z.infer<typeof ArchitectureSearchPageSchema>;
