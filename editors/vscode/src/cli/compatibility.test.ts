import { describe, expect, it } from "vitest";
import type { CapabilityReport } from "./contracts";
import { COMPASS_REQUIREMENTS, compatibilityIssue } from "./compatibility";

const compatible: CapabilityReport = {
  schema: "compass.ide.capabilities/1",
  compass_version: "0.1.4",
  contracts: {
    graph_viewer: "compass.viewer.graph/1",
    progress: "compass.ide.progress/1"
  },
  features: {
    graph: true
  }
};

describe("compatibilityIssue", () => {
  it("explains a failed capability negotiation instead of running an unsupported command", () => {
    expect(compatibilityIssue(
      undefined,
      "error: unknown command 'capabilities'",
      COMPASS_REQUIREMENTS.graph
    )).toContain("Capability negotiation failed: error: unknown command 'capabilities'");
  });

  it("rejects a mismatched versioned contract", () => {
    const report = {
      ...compatible,
      contracts: { ...compatible.contracts, graph_viewer: "compass.viewer.graph/2" }
    };
    expect(compatibilityIssue(report, undefined, COMPASS_REQUIREMENTS.graph))
      .toContain("requires 'compass.viewer.graph/1'");
  });

  it("accepts a matching feature and contract", () => {
    expect(compatibilityIssue(compatible, undefined, COMPASS_REQUIREMENTS.graph))
      .toBeUndefined();
  });
});
