import { describe, expect, it } from "vitest";
import { refreshedGraphState } from "./sessionRegistry";

describe("refreshedGraphState", () => {
  it("gives an active writer precedence over filesystem state", () => {
    expect(refreshedGraphState("available", true, true)).toBe("building");
    expect(refreshedGraphState("not-materialized", false, true)).toBe("building");
  });

  it("preserves a failed operation until a successful workflow changes it", () => {
    expect(refreshedGraphState("failed", false, false)).toBe("failed");
    expect(refreshedGraphState("failed", true, false)).toBe("failed");
  });

  it("uses graph materialization for stable non-failure states", () => {
    expect(refreshedGraphState("available", false, false)).toBe("not-materialized");
    expect(refreshedGraphState("not-materialized", true, false)).toBe("available");
  });
});
