import type {
  CompassDiscovery,
  CompassInstallation,
  CompassInstallationSource
} from "./discovery";

export type CompassSelectionItem = {
  label: string;
  description: string;
  detail: string;
  installation?: CompassInstallation | undefined;
  browse?: boolean | undefined;
};

export function compassSelectionItems(
  discovery: CompassDiscovery
): CompassSelectionItem[] {
  const activeExecutable = discovery.kind === "found"
    ? discovery.executable
    : undefined;
  const detected = discovery.installations.map((installation) => ({
    label: installation.version
      ? `$(terminal) Compass ${installation.version}`
      : "$(terminal) Compass (version unavailable)",
    description: installation.executable === activeExecutable
      ? "Current selection"
      : sourceLabel(installation.source),
    detail: installation.executable,
    installation
  }));
  return [
    ...detected,
    {
      label: "$(folder-opened) Browse for another Compass CLI…",
      description: "Choose an executable outside detected locations",
      detail: "The selected path will be stored in compass.cliPath",
      browse: true
    }
  ];
}

function sourceLabel(source: CompassInstallationSource): string {
  switch (source) {
    case "configured":
      return "Configured path";
    case "path":
      return "Detected on PATH";
    case "common":
      return "Common install location";
  }
}
