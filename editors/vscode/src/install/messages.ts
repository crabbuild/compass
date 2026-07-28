import type { CliOnboardingState } from "@compass/viewer";
import { z } from "zod";

const BoundedText = z.string().max(8192);
const StateSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("ready-to-install"),
    platform: BoundedText,
    command: BoundedText
  }).strict(),
  z.object({
    kind: z.literal("installing"),
    platform: BoundedText,
    command: BoundedText
  }).strict(),
  z.object({ kind: z.literal("verifying") }).strict(),
  z.object({
    kind: z.literal("ready"),
    version: BoundedText,
    executable: BoundedText,
    hasWorkspace: z.boolean()
  }).strict(),
  z.object({
    kind: z.literal("error"),
    title: BoundedText,
    message: BoundedText,
    searched: z.array(BoundedText).max(256).optional(),
    canVerifyAgain: z.boolean()
  }).strict(),
  z.object({
    kind: z.literal("unsupported"),
    platform: BoundedText,
    message: BoundedText
  }).strict()
]);

export const OnboardingToHostMessageSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("ready") }).strict(),
  z.object({ type: z.literal("install") }).strict(),
  z.object({ type: z.literal("verifyAgain") }).strict(),
  z.object({ type: z.literal("selectExisting") }).strict(),
  z.object({ type: z.literal("initializeRepository") }).strict(),
  z.object({ type: z.literal("openRepository") }).strict(),
  z.object({ type: z.literal("showTerminal") }).strict()
]);

export const HostToOnboardingMessageSchema = z.object({
  type: z.literal("state"),
  state: StateSchema
}).strict();

export type HostToOnboardingMessage = {
  type: "state";
  state: CliOnboardingState;
};
