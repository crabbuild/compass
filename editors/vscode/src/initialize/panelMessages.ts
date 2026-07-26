import { z } from "zod";
import type { InitializationRequest } from "@compass/viewer";

const PathRuleSchema = z.string()
  .trim()
  .min(1)
  .max(4096)
  .refine((value) => !value.includes("\0"));

const StartMessageSchema = z.object({
  type: z.literal("start"),
  request: z.object({
    includes: z.array(PathRuleSchema).max(256),
    excludes: z.array(PathRuleSchema).max(256),
    replaceExisting: z.boolean()
  })
});

export function parseInitializationRequest(message: unknown): InitializationRequest | undefined {
  const parsed = StartMessageSchema.safeParse(message);
  return parsed.success ? parsed.data.request : undefined;
}
