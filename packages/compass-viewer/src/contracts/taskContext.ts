import { z } from "zod";

import {
  CodeEvidenceSchema,
  CodeQueryResponseSchema,
  CodeSearchHitSchema,
  CodeNodeRoleSchema,
  CodeResolutionSchema,
  CodeSourceAnchorSchema
} from "./codeQuery";

export const TASK_CONTEXT_SCHEMA = "compass.task-context/2" as const;
export const FRAMEWORK_CONTEXT_SCHEMA = "compass.framework-context/1" as const;
const KNOWN_FRAMEWORK_PACK_IDS = [
  "react-ui", "nextjs-routes", "react-router-routes", "remix-routes",
  "tanstack-router", "tanstack-start", "vite-config", "typescript-web",
  "express-web", "fastify-web", "hono-web", "python-web", "go-web", "axum-web",
  "rust-web", "spring-java", "spring-kotlin", "rails-ruby", "php-frameworks",
  "aspnet-csharp", "vapor-swift", "filesystem-routes", "enterprise-domain-facts",
  "drupal-routing-config", "play-routes-config", "filesystem-template-routes"
] as const;

const frameworkText = z.string().min(1).max(16_384).refine(
  (value) => ![...value].some((character) => character.charCodeAt(0) < 32),
  "framework context text must not contain control characters"
);

const routeStageSchema = z.enum([
  "middleware", "layout", "template", "loading", "default", "error_boundary",
  "not_found", "boundary", "loader", "action", "handler", "data_loader",
  "route_component"
]);

const edgeKindSchema = z.enum([
  "contains", "embeds", "calls", "imports", "exports", "extends", "implements",
  "mixes_in", "references", "type_of", "returns", "instantiates", "overrides",
  "decorates", "routes_to", "reads", "writes", "aliases", "registers", "handles",
  "publishes", "subscribes", "produces", "consumes", "schedules", "triggers",
  "tests", "depends_on", "documents", "maps_to", "renders"
]);

const provenanceSchema = CodeEvidenceSchema;

const frameworkPackSchema = z.strictObject({
  id: z.enum(KNOWN_FRAMEWORK_PACK_IDS),
  version: z.number().int().positive(),
  qualification: z.enum(["qualified", "qualifying", "incomplete", "unsupported", "ambiguous"]),
  capabilities: z.array(frameworkText).max(64),
  observedNodes: z.number().int().nonnegative(),
  observedRelations: z.number().int().nonnegative()
});

const frameworkStageSchema = z.strictObject({
  stage: routeStageSchema,
  position: z.number().int().nonnegative(),
  reference: frameworkText,
  resolution: CodeResolutionSchema,
  source: CodeSourceAnchorSchema.nullable(),
  target: frameworkText.nullable(),
  provenance: z.array(provenanceSchema).max(256)
});

const frameworkRouteSchema = z.strictObject({
  nodeId: frameworkText,
  framework: frameworkText,
  operation: frameworkText,
  path: frameworkText,
  declaringScope: frameworkText,
  resolution: CodeResolutionSchema,
  stages: z.array(frameworkStageSchema).max(256),
  provenance: z.array(provenanceSchema).max(256)
});

const frameworkRelationSchema = z.strictObject({
  id: frameworkText,
  relation: edgeKindSchema,
  source: frameworkText,
  target: frameworkText,
  details: z.unknown().nullable(),
  relationshipSite: CodeSourceAnchorSchema.nullable(),
  provenance: z.array(provenanceSchema).max(256)
});

const frameworkBoundarySchema = z.strictObject({
  nodeId: frameworkText,
  framework: frameworkText,
  roles: z.array(CodeNodeRoleSchema).max(16),
  source: CodeSourceAnchorSchema.nullable(),
  provenance: z.array(provenanceSchema).max(256)
});

const frameworkStatusSchema = z.strictObject({
  framework: frameworkText,
  capability: frameworkText,
  reason: frameworkText
});

const frameworkAmbiguitySchema = z.strictObject({
  kind: frameworkText,
  reference: frameworkText,
  candidates: z.array(frameworkText).max(256)
});

export const FrameworkContextSchema = z.strictObject({
  schema: z.literal(FRAMEWORK_CONTEXT_SCHEMA),
  graphIdentity: frameworkText,
  buildGenerationIdentity: frameworkText,
  focusNodeId: frameworkText.nullable(),
  packs: z.array(frameworkPackSchema).max(256),
  routes: z.array(frameworkRouteSchema).max(256),
  relations: z.array(frameworkRelationSchema).max(256),
  renderedBy: z.array(frameworkRelationSchema).max(256),
  renders: z.array(frameworkRelationSchema).max(256),
  configDependencies: z.array(frameworkRelationSchema).max(256),
  runtimeBoundaries: z.array(frameworkBoundarySchema).max(256),
  unsupported: z.array(frameworkStatusSchema).max(256),
  incomplete: z.array(frameworkStatusSchema).max(256),
  ambiguities: z.array(frameworkAmbiguitySchema).max(256),
  truncated: z.boolean(),
  recordLimit: z.number().int().positive().max(256),
  byteLimit: z.number().int().positive().max(262_144)
});

const targetSchema = z.discriminatedUnion("state", [
  z.strictObject({ state: z.literal("exact"), nodeId: frameworkText }),
  z.strictObject({ state: z.literal("ambiguous"), candidates: z.array(CodeSearchHitSchema).max(256) }),
  z.strictObject({ state: z.literal("not_found"), candidates: z.array(CodeSearchHitSchema).max(256) })
]);

const taskSectionSchema = z.strictObject({
  kind: z.enum([
    "declaration_source", "exact_callers", "exact_callees", "implementation_type",
    "related_tests", "transitive_impact", "framework"
  ]),
  evidence: CodeQueryResponseSchema
});

const knowledgeSchema = z.strictObject({
  path: frameworkText,
  date: frameworkText,
  question: frameworkText,
  outcome: frameworkText,
  correction: z.string().max(16_384),
  sourceNodes: z.array(frameworkText).max(256),
  provenance: frameworkText
});

const omissionSchema = z.strictObject({ category: frameworkText, reason: frameworkText });

const digestSchema = z.string().regex(/^[0-9a-f]{64}$/);
const baseGenerationSchema = z.strictObject({
  generationId: frameworkText,
  graphDigest: digestSchema
});
const baseNodeRefSchema = z.strictObject({
  baseGeneration: baseGenerationSchema,
  id: frameworkText,
  kind: z.string().min(1).max(128),
  recordDigest: digestSchema
});
const baseEdgeRefSchema = z.strictObject({
  baseGeneration: baseGenerationSchema,
  id: frameworkText,
  kind: edgeKindSchema,
  source: frameworkText,
  target: frameworkText,
  recordDigest: digestSchema
});
const baseFactRefSchema = z.discriminatedUnion("factType", [
  baseNodeRefSchema.extend({ factType: z.literal("node") }).strict(),
  baseEdgeRefSchema.extend({ factType: z.literal("edge") }).strict()
]);
const groundingEvidenceSchema = z.discriminatedUnion("evidenceType", [
  z.strictObject({
    evidenceType: z.literal("source_span"), file: frameworkText,
    anchor: CodeSourceAnchorSchema, fileDigest: digestSchema, excerptDigest: digestSchema
  }),
  z.strictObject({
    evidenceType: z.literal("base_fact"), fact: baseFactRefSchema, recordDigest: digestSchema
  }),
  z.strictObject({
    evidenceType: z.literal("base_path"), nodes: z.array(baseNodeRefSchema).max(1001),
    edges: z.array(baseEdgeRefSchema).max(1000), pathDigest: digestSchema
  }),
  z.strictObject({
    evidenceType: z.literal("prior_assertion"), assertion: frameworkText,
    revision: digestSchema, assertionDigest: digestSchema
  }),
  z.strictObject({
    evidenceType: z.literal("snapshot_artifact"), artifact: frameworkText,
    artifactDigest: digestSchema, jsonPointer: z.string().max(16_384).optional()
  })
]);
const compositionOmissionsSchema = z.strictObject({
  total: z.number().int().nonnegative(),
  direct: z.number().int().nonnegative(),
  cascaded: z.number().int().nonnegative(),
  examples: z.array(z.strictObject({
    id: frameworkText,
    kind: z.enum(["node", "edge"]),
    reason: frameworkText,
    challenge: frameworkText.optional()
  })).max(256),
  omittedExamples: z.number().int().nonnegative()
});
export const AgentKnowledgeSchema = z.strictObject({
  schema: z.literal("compass.agent-knowledge/1"),
  effectiveIdentity: digestSchema,
  baseGeneration: baseGenerationSchema,
  overlayRevision: digestSchema,
  compositionProfile: z.enum(["augment", "curated"]),
  assertions: z.array(z.strictObject({
    assertionId: frameworkText,
    projectedId: frameworkText,
    owner: frameworkText,
    version: z.number().int().positive(),
    groundingStatus: z.literal("GROUNDED"),
    structuralConfidence: z.enum(["exact", "inferred", "ambiguous"]),
    certificateDigest: digestSchema,
    summary: z.string().max(16_384),
    citations: z.array(groundingEvidenceSchema).max(64)
  })).max(100),
  challenges: z.array(z.strictObject({
    challengeId: frameworkText,
    targetId: frameworkText,
    effect: z.enum(["flag", "mask"]),
    masked: z.boolean(),
    groundingStatus: z.literal("GROUNDED"),
    certificateDigest: digestSchema,
    summary: z.string().max(16_384),
    citations: z.array(groundingEvidenceSchema).max(64)
  })).max(100),
  omissions: compositionOmissionsSchema,
  truncated: z.boolean(),
  omittedRecords: z.number().int().nonnegative()
});

const workSchema = z.strictObject({
  schema: z.literal("compass.task-context-profile/1"),
  queryCount: z.number().int().nonnegative(),
  candidatesReturned: z.number().int().nonnegative(),
  nodesReturned: z.number().int().nonnegative(),
  edgesReturned: z.number().int().nonnegative(),
  filesVerified: z.number().int().nonnegative(),
  sourceBytes: z.number().int().nonnegative(),
  knowledgeItemsRead: z.number().int().nonnegative(),
  frameworkRecords: z.number().int().nonnegative(),
  frameworkBytes: z.number().int().nonnegative(),
  agentKnowledgeRecords: z.number().int().nonnegative(),
  agentKnowledgeBytes: z.number().int().nonnegative(),
  responseBytes: z.number().int().nonnegative()
});

export const TaskContextSchema = z.strictObject({
  schema: z.literal(TASK_CONTEXT_SCHEMA),
  intent: z.enum(["explain", "modify", "debug", "test"]),
  requestedTarget: frameworkText,
  target: targetSchema,
  graphIdentity: frameworkText,
  buildGenerationIdentity: frameworkText,
  sections: z.array(taskSectionSchema).max(32),
  framework: FrameworkContextSchema,
  agentKnowledge: AgentKnowledgeSchema.optional(),
  projectKnowledge: z.array(knowledgeSchema).max(100),
  omissions: z.array(omissionSchema).max(256),
  truncated: z.boolean(),
  work: workSchema,
  resultDigest: z.string().regex(/^sha256:[0-9a-f]{64}$/)
});

export function decodeTaskContext(value: unknown): TaskContext {
  return TaskContextSchema.parse(value);
}

export type FrameworkContext = z.infer<typeof FrameworkContextSchema>;
export type AgentKnowledge = z.infer<typeof AgentKnowledgeSchema>;
export type TaskContext = z.infer<typeof TaskContextSchema>;
export type TaskContextSection = z.infer<typeof taskSectionSchema>;
