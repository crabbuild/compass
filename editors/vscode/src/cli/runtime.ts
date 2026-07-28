import { COMPASS_REQUIREMENTS, compatibilityIssue } from "./compatibility";
import { CapabilityReportSchema, type CapabilityReport } from "./contracts";
import type { CompassDiscovery, CompassInstallation } from "./discovery";
import { CompassProcessManager } from "./processManager";
import type { RepositorySession } from "../workspace/repositorySession";

type CapabilityRunner = Pick<CompassProcessManager, "runJson">;

export type ActivatedCompass = {
  installation: CompassInstallation;
  capabilities: CapabilityReport;
};

export type CompassRuntimeDependencies = {
  processes: CompassProcessManager;
  sessions(): readonly RepositorySession[];
  persistCliPath(executable: string): Promise<void>;
  createCandidateProcesses?(executable: string): CapabilityRunner;
};

export class CompassRuntime {
  private current: CompassDiscovery;
  private readonly listeners = new Set<() => void>();

  constructor(
    discovery: CompassDiscovery,
    private readonly dependencies: CompassRuntimeDependencies
  ) {
    this.current = discovery;
  }

  get discovery(): CompassDiscovery {
    return this.current;
  }

  onDidChange(listener: () => void): { dispose(): void } {
    this.listeners.add(listener);
    return { dispose: () => this.listeners.delete(listener) };
  }

  async activate(installation: CompassInstallation): Promise<ActivatedCompass> {
    const sessions = this.dependencies.sessions();
    if (sessions.some((session) => session.activeWriter || session.watch)) {
      throw new Error("Stop active Compass builds and watchers before changing the CLI.");
    }

    const candidate = this.dependencies.createCandidateProcesses?.(
      installation.executable
    ) ?? new CompassProcessManager(installation.executable);
    const cwd = sessions[0]?.root ?? process.cwd();
    const capabilities = await candidate.runJson(
      cwd,
      ["capabilities", "--format", "json"],
      CapabilityReportSchema
    );
    const issue = Object.values(COMPASS_REQUIREMENTS)
      .map((requirement) => compatibilityIssue(capabilities, undefined, requirement))
      .find((value) => value !== undefined);
    if (issue) throw new Error(issue);

    const activatedInstallation: CompassInstallation = {
      ...installation,
      version: installation.version ?? capabilities.compass_version
    };
    await this.dependencies.persistCliPath(activatedInstallation.executable);
    this.dependencies.processes.useExecutable(activatedInstallation.executable);
    for (const session of sessions) {
      session.capabilities = capabilities;
      session.capabilityError = undefined;
    }
    this.current = activeDiscovery(activatedInstallation, this.current);
    for (const listener of this.listeners) listener();
    return { installation: activatedInstallation, capabilities };
  }
}

function activeDiscovery(
  installation: CompassInstallation,
  previous: CompassDiscovery
): CompassDiscovery {
  const installations = [
    installation,
    ...previous.installations.filter(
      (candidate) => candidate.executable !== installation.executable
    )
  ];
  return {
    kind: "found",
    executable: installation.executable,
    ...(installation.version ? { version: installation.version } : {}),
    installations,
    searched: previous.searched
  };
}
