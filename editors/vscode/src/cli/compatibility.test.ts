import { describe, expect, it } from "vitest";
import type { CapabilityReport } from "./contracts";
import {
  COMPASS_REQUIREMENTS,
  compatibilityIssue,
  isSupportedCompassVersion
} from "./compatibility";

const compatible: CapabilityReport = {
  schema: "compass.ide.capabilities/1",
  compass_version: "0.3.0",
  contracts: {
    graph_viewer: "compass.viewer.graph/1",
    progress: "compass.ide.progress/1"
  },
  features: {
    graph: true,
    community_detail: true
  }
};

describe("compatibilityIssue", () => {
  it("rejects Compass releases and prereleases below 0.3.0", () => {
    for (const version of [
      "0.1.9",
      "0.2.99",
      "0.3.0-beta.1",
      "00.3.0",
      "999999999999999999999.0.0",
      "invalid"
    ]) {
      expect(compatibilityIssue(
        { ...compatible, compass_version: version },
        undefined,
        COMPASS_REQUIREMENTS.graph
      )).toContain("requires Compass CLI 0.3.0 or newer");
    }
  });

  it("accepts stable 0.3.0 and later semantic versions", () => {
    expect(isSupportedCompassVersion("0.3.0")).toBe(true);
    expect(isSupportedCompassVersion("0.3.0+build.7")).toBe(true);
    expect(isSupportedCompassVersion("0.3.1-beta.1")).toBe(true);
    expect(isSupportedCompassVersion("1.0.0")).toBe(true);
  });

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

  it("hard-requires current community drill-down support for graph workflows", () => {
    expect(compatibilityIssue({
      ...compatible,
      features: { ...compatible.features, community_detail: false }
    }, undefined, COMPASS_REQUIREMENTS.graph))
      .toContain("'community_detail' feature");
  });

  it("requires the language-neutral call graph contract", () => {
    const report: CapabilityReport = {
      ...compatible,
      contracts: {
        ...compatible.contracts,
        call_graph: "compass.call_graph/1"
      },
      features: {
        ...compatible.features,
        call_graph: true
      }
    };

    expect(compatibilityIssue(report, undefined, COMPASS_REQUIREMENTS.calls))
      .toBeUndefined();
    expect(compatibilityIssue(compatible, undefined, COMPASS_REQUIREMENTS.calls))
      .toContain("'call_graph' feature");
  });

  it("requires the complete architecture flow contract", () => {
    const report: CapabilityReport = {
      ...compatible,
      contracts: {
        ...compatible.contracts,
        architecture_viewer: "compass.viewer.architecture/1"
      }
    };

    expect(compatibilityIssue(report, undefined, COMPASS_REQUIREMENTS.architecture))
      .toBeUndefined();
    expect(compatibilityIssue({
      ...report,
      contracts: {
        ...report.contracts,
        architecture_viewer: "compass.viewer.architecture/2"
      }
    }, undefined, COMPASS_REQUIREMENTS.architecture))
      .toContain("requires 'compass.viewer.architecture/1'");
  });
});
