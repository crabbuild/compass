import type { CapabilityReport } from "./contracts";

export const MINIMUM_COMPASS_VERSION = "0.3.0";

export type CapabilityRequirement = {
  workflow: string;
  features?: readonly string[];
  contracts?: Readonly<Record<string, string>>;
};

export const COMPASS_REQUIREMENTS = {
  initialize: {
    workflow: "initialize repositories",
    features: ["init"],
    contracts: { progress: "compass.ide.progress/1" }
  },
  update: {
    workflow: "update repositories",
    features: ["update"],
    contracts: { progress: "compass.ide.progress/1" }
  },
  watch: {
    workflow: "watch repositories",
    features: ["watch"],
    contracts: { progress: "compass.ide.progress/1" }
  },
  graph: {
    workflow: "open the code graph",
    features: ["graph", "community_detail"],
    contracts: { graph_viewer: "compass.viewer.graph/1" }
  },
  calls: {
    workflow: "open the call graph",
    features: ["call_graph"],
    contracts: { call_graph: "compass.call_graph/1" }
  },
  architecture: {
    workflow: "open the architecture flow",
    features: ["graph"],
    contracts: { architecture_viewer: "compass.viewer.architecture/1" }
  },
  query: {
    workflow: "query the codebase",
    features: ["query"]
  },
  history: {
    workflow: "open codebase evolution",
    features: ["history"],
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
  const versionIssue = minimumCompassVersionIssue(report.compass_version);
  if (versionIssue) return versionIssue;
  for (const feature of requirement.features ?? []) {
    if (report.features[feature] !== true) {
      return `Compass CLI ${report.compass_version} does not advertise the '${feature}' feature required to ${requirement.workflow}.`;
    }
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

export function minimumCompassVersionIssue(version: string): string | undefined {
  return isSupportedCompassVersion(version)
    ? undefined
    : `Compass CLI ${version || "(version unavailable)"} is unsupported. `
      + `This extension requires Compass CLI ${MINIMUM_COMPASS_VERSION} or newer.`;
}

export function isSupportedCompassVersion(version: string): boolean {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(
    version.trim()
  );
  if (!match) return false;
  const current = [Number(match[1]), Number(match[2]), Number(match[3])];
  if (current.some((value) => !Number.isSafeInteger(value))) return false;
  const minimum = [0, 3, 0];
  for (let index = 0; index < minimum.length; index += 1) {
    const value = current[index] ?? 0;
    const required = minimum[index] ?? 0;
    if (value > required) return true;
    if (value < required) return false;
  }
  return match[4] === undefined;
}
