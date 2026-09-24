import { z } from "zod";
import { GraphViewModelSchema } from "./graph";

export const HIERARCHY_VIEW_SCHEMA = "compass.viewer.hierarchy/1" as const;

/**
 * The rule that produced a group label. Readers switch on this instead of
 * parsing label text: a generic label says nothing about its members.
 */
export const HierarchyLabelRuleSchema = z.enum([
  "dominantDirectory",
  "modulePrefix",
  "hubMember",
  "communityId"
]);

/**
 * How a level's groups were derived from the level below. `relationship` merges
 * on projected evidence; `locationAffinity` merges on shared source location
 * after relationship evidence stopped reducing the level.
 */
export const LevelMergeSchema = z.enum(["relationship", "locationAffinity"]);

export const HierarchyGroupSchema = z.strictObject({
  index: z.number().int().nonnegative(),
  community: z.number().int().nonnegative().optional(),
  label: z.string().min(1),
  labelRule: HierarchyLabelRuleSchema,
  labelGeneric: z.boolean(),
  memberCount: z.number().int().positive(),
  childIndices: z.array(z.number().int().nonnegative()).default([]),
  cohesion: z.number(),
  conductance: z.number(),
  boundaryKinds: z.record(z.string(), z.number().int().nonnegative()).default({}),
  detailAvailable: z.boolean()
});

export const HierarchyLevelSchema = z.strictObject({
  level: z.number().int().nonnegative(),
  merge: LevelMergeSchema,
  resolution: z.number().optional(),
  groupCount: z.number().int().positive(),
  memberCount: z.number().int().nonnegative(),
  groups: z.array(HierarchyGroupSchema).min(1),
  /** Omitted when the level holds more groups than the export's node budget. */
  model: GraphViewModelSchema.optional()
});

/**
 * The published community hierarchy as an export embeds it. Level 0 is the
 * root a reader opens; the last level is the published partition. Levels are
 * defined by `childIndices` into the level below, never by repeating node ids.
 */
export const CommunityHierarchyViewSchema = z.strictObject({
  schema: z.literal(HIERARCHY_VIEW_SCHEMA),
  budgetIdentity: z.string().min(1),
  mergePolicy: z.string().min(1),
  rootTarget: z.number().int().positive(),
  levelTarget: z.number().int().positive(),
  maxLevels: z.number().int().positive(),
  budgetSatisfied: z.boolean(),
  finestCommunityCount: z.number().int().nonnegative(),
  finestSignature: z.string().min(1),
  boundaryKinds: z.array(z.string()).default([]),
  levels: z.array(HierarchyLevelSchema).min(1)
});

export type HierarchyLabelRule = z.infer<typeof HierarchyLabelRuleSchema>;
export type LevelMerge = z.infer<typeof LevelMergeSchema>;
export type HierarchyGroup = z.infer<typeof HierarchyGroupSchema>;
export type HierarchyLevel = z.infer<typeof HierarchyLevelSchema>;
export type CommunityHierarchyView = z.infer<typeof CommunityHierarchyViewSchema>;
