import { z } from "zod";

export const CapabilityReportSchema = z.object({
  schema: z.literal("compass.ide.capabilities/1"),
  compass_version: z.string(),
  contracts: z.record(z.string(), z.string()),
  features: z.record(z.string(), z.boolean())
});
export type CapabilityReport = z.infer<typeof CapabilityReportSchema>;

export const ProgressEventSchema = z.object({
  schema: z.literal("compass.ide.progress/1"),
  operation_id: z.string(),
  operation: z.string(),
  state: z.enum(["started", "running", "retrying", "succeeded", "failed", "cancelled"]),
  phase: z.string(),
  current: z.number().nullable().optional(),
  total: z.number().nullable().optional(),
  message: z.string(),
  terminal: z.boolean()
});
export type ProgressEvent = z.infer<typeof ProgressEventSchema>;
