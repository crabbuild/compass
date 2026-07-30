import { GraphViewModelSchema, SourceLocationSchema } from "@compass/viewer/contracts/graph";
import { CodeQueryResponseSchema } from "@compass/viewer/contracts/codeQuery";
import { z } from "zod";

export const HostToGraphMessageSchema = z.discriminatedUnion("type", [
  z.strictObject({
    type: z.literal("hydrateGraph"),
    requestId: z.string(),
    repositoryId: z.string(),
    model: GraphViewModelSchema
  }),
  z.strictObject({
    type: z.literal("graphLoadStatus"),
    mode: z.literal("large"),
    graphBytes: z.number().int().nonnegative(),
    phase: z.enum(["snapshotting", "exporting"])
  }),
  z.strictObject({
    type: z.literal("communityGraph"),
    requestId: z.string(),
    repositoryId: z.string(),
    communityId: z.number().int().nonnegative(),
    model: GraphViewModelSchema
  }),
  z.strictObject({
    type: z.literal("communityError"),
    requestId: z.string(),
    communityId: z.number().int().nonnegative(),
    message: z.string()
  }),
  z.strictObject({
    type: z.literal("codeQueryResult"),
    requestId: z.string(),
    repositoryId: z.string(),
    result: CodeQueryResponseSchema
  }),
  z.strictObject({ type: z.literal("error"), message: z.string() })
]);

export const GraphToHostMessageSchema = z.discriminatedUnion("type", [
  z.strictObject({ type: z.literal("ready") }),
  z.strictObject({ type: z.literal("retry") }),
  z.strictObject({ type: z.literal("showOutput") }),
  z.strictObject({
    type: z.literal("openSource"),
    repositoryId: z.string(),
    source: SourceLocationSchema
  }),
  z.strictObject({
    type: z.literal("openCommunity"),
    requestId: z.string(),
    repositoryId: z.string(),
    communityId: z.number().int().nonnegative()
  }),
  z.strictObject({
    type: z.literal("runCodeQuery"),
    repositoryId: z.string(),
    operation: z.enum(["callers", "callees", "impact"]),
    symbol: z.string().min(1)
  })
]);
