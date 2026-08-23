import { z } from "zod";
import { CallGraphResponseSchema } from "./callGraph";
import { ArchitectureViewModelSchema } from "./architecture";
import { CodeQueryResponseSchema } from "./codeQuery";
import { GraphViewModelSchema } from "./graph";

export const WORKBENCH_SCHEMA = "compass.viewer.workbench/1" as const;

export const WorkbenchCoverageSchema = z.strictObject({
  status: z.enum(["complete", "summary", "partial"]),
  truncated: z.boolean(),
  nodes: z.number().int().nonnegative(),
  edges: z.number().int().nonnegative(),
  limitations: z.array(z.string()).default([])
});

const ViewBase = {
  id: z.string().min(1),
  title: z.string().min(1),
  description: z.string(),
  coverage: WorkbenchCoverageSchema
};

export const ArtifactLensSchema = z.enum([
  "dependencies",
  "routes",
  "data",
  "messaging",
  "tests",
  "provenance"
]);

export const WorkbenchViewSchema = z.discriminatedUnion("kind", [
  z.strictObject({
    ...ViewBase,
    kind: z.literal("code"),
    model: GraphViewModelSchema,
    communityDetails: z.record(z.string(), GraphViewModelSchema).default({})
  }),
  z.strictObject({
    ...ViewBase,
    kind: z.literal("call"),
    root: z.string(),
    graph: CallGraphResponseSchema
  }),
  z.strictObject({
    ...ViewBase,
    kind: z.literal("impact"),
    root: z.string(),
    result: CodeQueryResponseSchema
  }),
  z.strictObject({
    ...ViewBase,
    kind: z.literal("architecture"),
    model: ArchitectureViewModelSchema
  }),
  z.strictObject({
    ...ViewBase,
    kind: z.literal("history"),
    baseRevision: z.string(),
    targetRevision: z.string(),
    before: GraphViewModelSchema,
    after: GraphViewModelSchema
  }),
  z.strictObject({
    ...ViewBase,
    kind: z.literal("affected"),
    root: z.string(),
    relations: z.array(z.string()),
    depth: z.number().int().nonnegative(),
    model: GraphViewModelSchema
  }),
  z.strictObject({
    ...ViewBase,
    kind: z.literal("artifact"),
    lens: ArtifactLensSchema,
    relations: z.array(z.string()),
    model: GraphViewModelSchema
  })
]);

export const WorkbenchModelSchema = z.strictObject({
  schema: z.literal(WORKBENCH_SCHEMA),
  title: z.string(),
  graphIdentity: z.string().min(1),
  defaultView: z.string(),
  views: z.array(WorkbenchViewSchema).min(1)
}).superRefine((model, context) => {
  const ids = new Set<string>();
  for (const [index, view] of model.views.entries()) {
    if (ids.has(view.id)) {
      context.addIssue({
        code: "custom",
        path: ["views", index, "id"],
        message: `duplicate workbench view id '${view.id}'`
      });
    }
    ids.add(view.id);
  }
  if (!ids.has(model.defaultView)) {
    context.addIssue({
      code: "custom",
      path: ["defaultView"],
      message: "defaultView must identify one included view"
    });
  }
});

export type ArtifactLens = z.infer<typeof ArtifactLensSchema>;
export type WorkbenchCoverage = z.infer<typeof WorkbenchCoverageSchema>;
export type WorkbenchView = z.infer<typeof WorkbenchViewSchema>;
export type WorkbenchModel = z.infer<typeof WorkbenchModelSchema>;
