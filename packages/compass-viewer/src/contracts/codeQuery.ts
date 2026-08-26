import { z } from "zod";

export const CODE_QUERY_SCHEMA = "compass.query/1" as const;

export const CODE_QUERY_NODE_KINDS = [
  "file", "module", "package", "namespace", "class", "struct", "interface",
  "trait", "protocol", "enum", "enum_member", "type_alias", "function",
  "method", "constructor", "property", "field", "variable", "constant",
  "parameter", "import", "export", "macro", "annotation", "route",
  "component", "event", "message", "topic", "queue", "job", "resource",
  "schema", "query", "migration", "config_key", "database", "database_schema",
  "database_table", "database_view", "database_column", "database_index",
  "database_constraint", "database_procedure", "database_trigger"
] as const;

export const CODE_QUERY_EDGE_KINDS = [
  "contains", "calls", "imports", "exports", "extends", "implements",
  "references", "type_of", "returns", "instantiates", "overrides", "decorates",
  "routes_to", "reads", "writes", "aliases", "registers", "handles",
  "publishes", "subscribes", "produces", "consumes", "schedules", "triggers",
  "tests", "depends_on", "documents", "maps_to", "renders"
] as const;

export const CODE_QUERY_NODE_ROLES = [
  "controller", "route_handler", "middleware", "service", "resolver",
  "consumer", "producer", "subscriber", "repository", "model", "test",
  "fixture", "generated", "ui_component", "hook", "client_boundary",
  "client_component", "server_component", "server_function", "data_loader"
] as const;

export const CODE_QUERY_OPERATIONS = [
  "search", "callers", "callees", "impact", "explore", "node_trail"
] as const;

export const CODE_QUERY_ROUTE_STAGES = [
  "middleware", "dependency", "security", "layout", "template", "loading",
  "default", "error_boundary", "not_found", "boundary", "loader", "action",
  "handler", "data_loader", "route_component"
] as const;

export const CODE_QUERY_CONTRACT_MANIFEST = {
  schema: CODE_QUERY_SCHEMA,
  enums: {
    operation: CODE_QUERY_OPERATIONS,
    nodeKind: CODE_QUERY_NODE_KINDS,
    nodeRole: CODE_QUERY_NODE_ROLES,
    edgeKind: CODE_QUERY_EDGE_KINDS,
    routeStage: CODE_QUERY_ROUTE_STAGES,
    evidenceLayer: ["structural_graph", "program_ir"],
    evidenceOrigin: ["ast", "config", "convention", "artifact", "heuristic"],
    confidence: ["exact", "inferred", "ambiguous"],
    resolution: ["exact", "ambiguous", "unresolved"],
    diagnosticCode: [
      "no_match", "ambiguous_match", "direction_mismatch", "unresolved_handler", "incomplete_coverage",
      "stale_source_digest", "bounded_truncation", "program_orphan",
      "program_conflict", "program_unavailable"
    ]
  },
  fields: {
    response: [
      "schema", "operation", "results", "nodes", "edges", "files", "paths",
      "diagnostics", "limits", "truncated"
    ],
    limits: [
      "maxDepth", "maxNodes", "maxEdges", "maxPaths", "maxCandidates",
      "maxSourceBytes", "maxResponseBytes"
    ],
    sourceAnchor: [
      "file", "startByte", "endByte", "startLine", "startColumn", "endLine",
      "endColumn"
    ],
    evidence: [
      "layer", "origin", "extractor", "confidence", "anchor", "rule",
      "wiringSite", "resolution", "candidates"
    ],
    candidate: ["nodeId", "reason", "confidence", "score", "anchor"],
    node: [
      "id", "kind", "roles", "name", "qualifiedName", "language", "framework",
      "source", "details", "evidence"
    ],
    edge: [
      "id", "source", "target", "kind", "relationshipSite", "details", "evidence"
    ],
    file: ["path", "contentDigest", "source", "truncated"],
    path: [
      "id", "nodeIds", "edgeIds", "weakestResolution", "weakestConfidence"
    ],
    diagnostic: ["code", "message", "nodeId", "path"]
  }
} as const;

const portablePath = z.string().min(1).refine((value) => {
  if (value.includes("\\") || value.startsWith("/") || /^[A-Za-z]:/.test(value)) return false;
  return value.split("/").every((part) => part !== "" && part !== "." && part !== "..");
}, "source path must be a safe repository-relative path");

export const CodeSourceAnchorSchema = z.strictObject({
  file: portablePath,
  startByte: z.number().int().nonnegative(),
  endByte: z.number().int().nonnegative(),
  startLine: z.number().int().positive(),
  startColumn: z.number().int().nonnegative(),
  endLine: z.number().int().positive(),
  endColumn: z.number().int().nonnegative()
}).refine(
  (anchor) => anchor.endByte >= anchor.startByte
    && (anchor.endLine > anchor.startLine
      || (anchor.endLine === anchor.startLine && anchor.endColumn >= anchor.startColumn)),
  "source anchor must be ordered"
);

export const CodeNodeKindSchema = z.enum(CODE_QUERY_NODE_KINDS);
export const CodeEdgeKindSchema = z.enum(CODE_QUERY_EDGE_KINDS);
export const CodeNodeRoleSchema = z.enum(CODE_QUERY_NODE_ROLES);
export const CodeQueryOperationSchema = z.enum(CODE_QUERY_OPERATIONS);
export const CodeEvidenceLayerSchema = z.enum(["structural_graph", "program_ir"]);
export const CodeEvidenceOriginSchema = z.enum([
  "ast", "config", "convention", "artifact", "heuristic"
]);
export const CodeEvidenceConfidenceSchema = z.enum(["exact", "inferred", "ambiguous"]);
export const CodeResolutionSchema = z.enum(["exact", "ambiguous", "unresolved"]);

export const CodeResolutionCandidateSchema = z.strictObject({
  nodeId: z.string().min(1),
  reason: z.string().min(1),
  confidence: CodeEvidenceConfidenceSchema,
  score: z.number().finite().nullable().optional(),
  anchor: CodeSourceAnchorSchema.nullable().optional()
});

export const CodeEvidenceSchema = z.strictObject({
  layer: CodeEvidenceLayerSchema,
  origin: CodeEvidenceOriginSchema,
  extractor: z.string().min(1),
  confidence: CodeEvidenceConfidenceSchema,
  anchor: CodeSourceAnchorSchema.nullable(),
  rule: z.string().min(1).nullable(),
  wiringSite: CodeSourceAnchorSchema.nullable(),
  resolution: CodeResolutionSchema,
  candidates: z.array(CodeResolutionCandidateSchema).max(20).default([])
});

const FileNodeDetailsSchema = z.strictObject({
  type: z.literal("file"),
  data: z.strictObject({
    contentDigest: z.string().min(1),
    byteSize: z.number().int().nonnegative(),
    generated: z.boolean()
  })
});
const SymbolNodeDetailsSchema = z.strictObject({
  type: z.literal("symbol"),
  data: z.strictObject({
    signature: z.string().nullable().optional(),
    modifiers: z.array(z.string()).default([]),
    overloadDiscriminator: z.string().nullable().optional(),
    declaringType: z.string().nullable().optional(),
    signatureDigest: z.string().nullable().optional(),
    implementationDigest: z.string().nullable().optional(),
    sourceDigest: z.string().nullable().optional()
  })
});
const ImportExportNodeDetailsSchema = z.strictObject({
  type: z.literal("import_export"),
  data: z.strictObject({
    specifier: z.string(),
    importedName: z.string().nullable().optional(),
    localName: z.string().nullable().optional(),
    typeOnly: z.boolean()
  })
});
export const CodeRouteStageSchema = z.enum(CODE_QUERY_ROUTE_STAGES);
const RouteStageDetailsSchema = z.strictObject({
  stage: CodeRouteStageSchema,
  position: z.number().int().nonnegative(),
  reference: z.string().min(1),
  resolution: CodeResolutionSchema,
  sourceAnchor: CodeSourceAnchorSchema.nullable(),
  target: z.string().nullable().optional(),
  candidates: z.array(CodeResolutionCandidateSchema).max(20).default([])
});
const RouteNodeDetailsSchema = z.strictObject({
  type: z.literal("route"),
  data: z.strictObject({
    operation: z.string(),
    path: z.string(),
    originalPath: z.string().nullable().optional(),
    declaringScope: z.string(),
    resolution: CodeResolutionSchema,
    middlewareCount: z.number().int().nonnegative(),
    stages: z.array(RouteStageDetailsSchema).max(256).default([])
  })
});
const ComponentNodeDetailsSchema = z.strictObject({
  type: z.literal("component"),
  data: z.strictObject({ componentType: z.string() })
});
const ResourceNodeDetailsSchema = z.strictObject({
  type: z.literal("resource"),
  data: z.strictObject({
    resourceKind: z.enum(["document", "paper", "image", "concept", "rationale"]),
    uri: z.string().nullable().optional(),
    mediaType: z.string().nullable().optional()
  })
});
const MessagingNodeDetailsSchema = z.strictObject({
  type: z.literal("messaging"),
  data: z.strictObject({
    transport: z.string(),
    subject: z.string(),
    declaringScope: z.string()
  })
});
const JobNodeDetailsSchema = z.strictObject({
  type: z.literal("job"),
  data: z.strictObject({
    schedule: z.string().nullable().optional(),
    queue: z.string().nullable().optional()
  })
});
const SchemaNodeDetailsSchema = z.strictObject({
  type: z.literal("schema"),
  data: z.strictObject({
    dialect: z.string().nullable().optional(),
    logicalDatabase: z.string().nullable().optional(),
    namespace: z.string().nullable().optional()
  })
});
const QueryNodeDetailsSchema = z.strictObject({
  type: z.literal("query"),
  data: z.strictObject({
    dialect: z.string().nullable().optional(),
    operation: z.string().nullable().optional(),
    textDigest: z.string().nullable().optional()
  })
});
const ConfigNodeDetailsSchema = z.strictObject({
  type: z.literal("config"),
  data: z.strictObject({ format: z.string(), keyPath: z.string() })
});
const DatabaseNodeDetailsSchema = z.strictObject({
  type: z.literal("database"),
  data: z.strictObject({
    logicalDatabase: z.string(),
    databaseSchema: z.string().nullable().optional()
  })
});

export const CodeNodeDetailsSchema = z.discriminatedUnion("type", [
  FileNodeDetailsSchema,
  SymbolNodeDetailsSchema,
  ImportExportNodeDetailsSchema,
  RouteNodeDetailsSchema,
  ComponentNodeDetailsSchema,
  ResourceNodeDetailsSchema,
  MessagingNodeDetailsSchema,
  JobNodeDetailsSchema,
  SchemaNodeDetailsSchema,
  QueryNodeDetailsSchema,
  ConfigNodeDetailsSchema,
  DatabaseNodeDetailsSchema
]);

const CallEdgeDetailsSchema = z.strictObject({
  type: z.literal("call"),
  data: z.strictObject({
    dispatch: z.enum(["static", "virtual", "dynamic"]),
    receiverType: z.string().nullable().optional(),
    argumentCount: z.number().int().nonnegative().nullable().optional()
  })
});
const RouteEdgeDetailsSchema = z.strictObject({
  type: z.literal("route"),
  data: z.strictObject({
    stage: CodeRouteStageSchema,
    position: z.number().int().nonnegative().nullable().optional(),
    operation: z.string().nullable().optional()
  })
});
const MessagingEdgeDetailsSchema = z.strictObject({
  type: z.literal("messaging"),
  data: z.strictObject({ transport: z.string(), subject: z.string() })
});
const ScheduleEdgeDetailsSchema = z.strictObject({
  type: z.literal("schedule"),
  data: z.strictObject({ expression: z.string().nullable().optional() })
});
const MappingEdgeDetailsSchema = z.strictObject({
  type: z.literal("mapping"),
  data: z.strictObject({ mappingKind: z.string() })
});
const RenderEdgeDetailsSchema = z.strictObject({
  type: z.literal("render"),
  data: z.strictObject({
    renderKind: z.enum(["jsx", "create_element", "root", "lazy", "dynamic"]),
    boundary: z.string().nullable().optional()
  })
});

export const CodeEdgeDetailsSchema = z.discriminatedUnion("type", [
  CallEdgeDetailsSchema,
  RouteEdgeDetailsSchema,
  MessagingEdgeDetailsSchema,
  ScheduleEdgeDetailsSchema,
  MappingEdgeDetailsSchema,
  RenderEdgeDetailsSchema
]);

export const CodeQueryLimitsSchema = z.strictObject({
  maxDepth: z.number().int().positive(),
  maxNodes: z.number().int().positive(),
  maxEdges: z.number().int().positive(),
  maxPaths: z.number().int().positive(),
  maxCandidates: z.number().int().positive(),
  maxSourceBytes: z.number().int().positive(),
  maxResponseBytes: z.number().int().positive()
});

export const CodeSearchHitSchema = z.strictObject({
  nodeId: z.string().min(1),
  score: z.number().finite(),
  matchedFields: z.array(z.string())
});

export const CodeQueryNodeSchema = z.strictObject({
  id: z.string().min(1),
  kind: CodeNodeKindSchema,
  roles: z.array(CodeNodeRoleSchema),
  name: z.string().min(1),
  qualifiedName: z.string().min(1),
  language: z.string().nullable(),
  framework: z.string().nullable(),
  source: CodeSourceAnchorSchema.nullable(),
  details: CodeNodeDetailsSchema.nullable(),
  evidence: z.array(CodeEvidenceSchema)
});

export const CodeQueryEdgeSchema = z.strictObject({
  id: z.string().min(1),
  source: z.string().min(1),
  target: z.string().min(1),
  kind: CodeEdgeKindSchema,
  relationshipSite: CodeSourceAnchorSchema.nullable(),
  details: CodeEdgeDetailsSchema.nullable(),
  evidence: z.array(CodeEvidenceSchema)
});

export const CodeQueryFileSchema = z.strictObject({
  path: portablePath,
  contentDigest: z.string().min(1),
  source: z.string().nullable(),
  truncated: z.boolean()
});

export const CodeQueryPathSchema = z.strictObject({
  id: z.string().min(1),
  nodeIds: z.array(z.string().min(1)),
  edgeIds: z.array(z.string().min(1)),
  weakestResolution: CodeResolutionSchema,
  weakestConfidence: CodeEvidenceConfidenceSchema
});

export const CodeQueryDiagnosticSchema = z.strictObject({
  code: z.enum([
    "no_match", "ambiguous_match", "direction_mismatch", "unresolved_handler", "incomplete_coverage",
    "stale_source_digest", "bounded_truncation", "program_orphan",
    "program_conflict", "program_unavailable"
  ]),
  message: z.string().min(1),
  nodeId: z.string().nullable(),
  path: portablePath.nullable()
});

export const CodeQueryResponseSchema = z.strictObject({
  schema: z.literal(CODE_QUERY_SCHEMA),
  operation: CodeQueryOperationSchema,
  results: z.array(CodeSearchHitSchema),
  nodes: z.array(CodeQueryNodeSchema),
  edges: z.array(CodeQueryEdgeSchema),
  files: z.array(CodeQueryFileSchema),
  paths: z.array(CodeQueryPathSchema),
  diagnostics: z.array(CodeQueryDiagnosticSchema),
  limits: CodeQueryLimitsSchema,
  truncated: z.boolean()
});

export function decodeCodeQueryResponse(value: unknown): CodeQueryResponse {
  return CodeQueryResponseSchema.parse(value);
}

export type CodeQueryResponse = z.infer<typeof CodeQueryResponseSchema>;
export type CodeQueryOperation = z.infer<typeof CodeQueryOperationSchema>;
export type CodeQueryNode = z.infer<typeof CodeQueryNodeSchema>;
export type CodeQueryEdge = z.infer<typeof CodeQueryEdgeSchema>;
export type CodeQueryFile = z.infer<typeof CodeQueryFileSchema>;
export type CodeQueryPath = z.infer<typeof CodeQueryPathSchema>;
export type CodeQueryDiagnostic = z.infer<typeof CodeQueryDiagnosticSchema>;
export type CodeEvidenceRecord = z.infer<typeof CodeEvidenceSchema>;
export type CodeSourceAnchor = z.infer<typeof CodeSourceAnchorSchema>;
