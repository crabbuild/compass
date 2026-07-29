import type { CapabilityReport } from "./contracts";

export type CapabilityRequirement = {
  workflow: string;
  feature?: string;
  contracts?: Readonly<Record<string, string>>;
};

export const COMPASS_REQUIREMENTS = {
  initialize: {
    workflow: "initialize repositories",
    feature: "init",
    contracts: { progress: "compass.ide.progress/1" }
  },
  update: {
    workflow: "update repositories",
    feature: "update",
    contracts: { progress: "compass.ide.progress/1" }
  },
  watch: {
    workflow: "watch repositories",
    feature: "watch",
    contracts: { progress: "compass.ide.progress/1" }
  },
  graph: {
    workflow: "open the code graph",
    feature: "graph",
    contracts: { graph_viewer: "compass.viewer.graph/1" }
  },
  calls: {
    workflow: "open the call graph",
    feature: "call_graph",
    contracts: { call_graph: "compass.call_graph/1" }
  },
  architecture: {
    workflow: "open the architecture flow",
    feature: "graph",
    contracts: { callflow_viewer: "compass.viewer.callflow/2" }
  },
  query: {
    workflow: "query the codebase",
    feature: "query"
  },
  history: {
    workflow: "open codebase evolution",
    feature: "history",
    contracts: {
      history_timeline: "compass.history.timeline/1",
      history_change_counts: "compass.history.change_counts/1",
      history_viewer_graph: "compass.history.viewer_graph/1"
    }
  }
} as const satisfies Record<string, CapabilityRequirement>;

export function compatibilityIssue(
  report: CapabilityReport | undefined,
  negotiationError: string | undefined,
  requirement: CapabilityRequirement
): string | undefined {
  if (!report) {
    const detail = negotiationError ? ` Capability negotiation failed: ${negotiationError}` : "";
    return `The installed Compass CLI cannot ${requirement.workflow} with this extension.${detail}`;
  }
  if (requirement.feature && report.features[requirement.feature] !== true) {
    return `Compass CLI ${report.compass_version} does not advertise the '${requirement.feature}' feature required to ${requirement.workflow}.`;
  }
  for (const [name, expected] of Object.entries(requirement.contracts ?? {})) {
    const actual = report.contracts[name];
    if (actual !== expected) {
      return actual
        ? `Compass CLI ${report.compass_version} advertises ${name} as '${actual}', but this extension requires '${expected}' to ${requirement.workflow}.`
        : `Compass CLI ${report.compass_version} does not advertise '${expected}', which is required to ${requirement.workflow}.`;
    }
  }
  return undefined;
}
