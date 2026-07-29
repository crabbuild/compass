import {
  ArchitectureEvidenceSchema,
  ArchitectureOverviewSchema,
  ArchitectureRoutePageSchema,
  ArchitectureScopeSchema,
  ArchitectureSearchPageSchema,
  ArchitectureSectionPageSchema
} from "@compass/viewer/contracts/architecture";
import { z } from "zod";

const RequestIdentitySchema = z.object({
  requestId: z.string().min(1),
  repositoryId: z.string().min(1)
});

const DataRequestSchema = RequestIdentitySchema.extend({
  generation: z.number().int().nonnegative(),
  scope: ArchitectureScopeSchema,
  evidence: ArchitectureEvidenceSchema
});

const PageRequestSchema = DataRequestSchema.extend({
  page: z.number().int().positive(),
  pageSize: z.number().int().min(1).max(100),
  query: z.string().optional()
});

export const ArchitectureToHostMessageSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("ready") }),
  z.object({ type: z.literal("retry") }),
  z.object({ type: z.literal("showOutput") }),
  RequestIdentitySchema.extend({
    type: z.literal("setArchitectureFilters"),
    scope: ArchitectureScopeSchema,
    evidence: ArchitectureEvidenceSchema
  }),
  PageRequestSchema.extend({
    type: z.literal("requestSection"),
    sectionId: z.string().min(1),
    kind: z.enum(["symbols", "calls"])
  }),
  PageRequestSchema.extend({
    type: z.literal("requestRoute"),
    routeId: z.string().min(1)
  }),
  PageRequestSchema.extend({
    type: z.literal("searchArchitecture"),
    query: z.string()
  }),
  RequestIdentitySchema.extend({
    type: z.literal("openSource"),
    file: z.string().min(1)
  })
]);

const ResponseIdentitySchema = RequestIdentitySchema.extend({
  generation: z.number().int().nonnegative()
});

export const HostToArchitectureMessageSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("architectureLoading"),
    phase: z.enum(["exporting", "validating", "indexing", "mapping"]),
    message: z.string()
  }),
  ResponseIdentitySchema.extend({
    type: z.literal("architectureOverview"),
    model: ArchitectureOverviewSchema
  }),
  ResponseIdentitySchema.extend({
    type: z.literal("architectureSectionPage"),
    model: ArchitectureSectionPageSchema
  }),
  ResponseIdentitySchema.extend({
    type: z.literal("architectureRoutePage"),
    model: ArchitectureRoutePageSchema
  }),
  ResponseIdentitySchema.extend({
    type: z.literal("architectureSearchResults"),
    model: ArchitectureSearchPageSchema
  }),
  z.object({
    type: z.literal("error"),
    message: z.string(),
    recoverable: z.boolean().default(true)
  })
]);

export type ArchitectureToHostMessage = z.infer<typeof ArchitectureToHostMessageSchema>;
export type HostToArchitectureMessage = z.infer<typeof HostToArchitectureMessageSchema>;
