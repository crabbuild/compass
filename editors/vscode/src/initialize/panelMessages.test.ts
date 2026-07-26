import { describe, expect, it } from "vitest";
import { parseInitializationRequest } from "./panelMessages";

describe("parseInitializationRequest", () => {
  it("accepts bounded path rules and rejects malformed build requests", () => {
    expect(parseInitializationRequest({
      type: "start",
      request: {
        includes: ["src", "packages/**"],
        excludes: ["**/generated/**"],
        replaceExisting: true
      }
    })).toEqual({
      includes: ["src", "packages/**"],
      excludes: ["**/generated/**"],
      replaceExisting: true
    });
    expect(parseInitializationRequest({
      type: "start",
      request: { includes: ["src\0private"], excludes: [], replaceExisting: false }
    })).toBeUndefined();
    expect(parseInitializationRequest({
      type: "start",
      request: { includes: "src", excludes: [], replaceExisting: false }
    })).toBeUndefined();
    expect(parseInitializationRequest({
      type: "start",
      request: { includes: [], excludes: [] }
    })).toBeUndefined();
  });
});
