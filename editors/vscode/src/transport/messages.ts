import { GraphViewModelSchema, SourceLocationSchema } from "@compass/viewer/contracts/graph";
import { z } from "zod";

export const HostToGraphMessageSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("hydrateGraph"),
    requestId: z.string(),
    repositoryId: z.string(),
    model: GraphViewModelSchema
  }),
  z.object({
    type: z.literal("communityGraph"),
    requestId: z.string(),
    repositoryId: z.string(),
    communityId: z.number().int().nonnegative(),
    model: GraphViewModelSchema
  }),
  z.object({
    type: z.literal("communityError"),
    requestId: z.string(),
    communityId: z.number().int().nonnegative(),
    message: z.string()
  }),
  z.object({ type: z.literal("error"), message: z.string() })
]);

export const GraphToHostMessageSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("ready") }),
  z.object({ type: z.literal("retry") }),
  z.object({ type: z.literal("showOutput") }),
  z.object({
    type: z.literal("openSource"),
    repositoryId: z.string(),
    source: SourceLocationSchema
  }),
  z.object({
    type: z.literal("openCommunity"),
    requestId: z.string(),
    repositoryId: z.string(),
    communityId: z.number().int().nonnegative()
  })
]);
