import { describe, expect, it, vi } from "vitest";
import type { CapabilityReport } from "./contracts";
import type { CompassDiscovery, CompassInstallation } from "./discovery";
import { CompassProcessManager } from "./processManager";
import { CompassRuntime } from "./runtime";
import type { RepositorySession } from "../workspace/repositorySession";

const capabilities: CapabilityReport = {
  schema: "compass.ide.capabilities/1",
  compass_version: "0.3.0",
  contracts: {
    progress: "compass.ide.progress/1",
    graph_viewer: "compass.viewer.graph/1",
    call_graph: "compass.call_graph/1",
    callflow_viewer: "compass.viewer.callflow/1",
    history_timeline: "compass.history.timeline/1",
    history_change_counts: "compass.history.change_counts/1",
    history_viewer_graph: "compass.history.viewer_graph/1",
    semantic_diff_report: "compass.semantic_diff.report/1"
  },
  features: {
    init: true,
    update: true,
    watch: true,
    graph: true,
    community_detail: true,
    call_graph: true,
    query: true,
    history: true
  }
};

const missing: CompassDiscovery = {
  kind: "missing",
  installations: [],
  searched: ["/usr/bin/compass"]
};

const installation: CompassInstallation = {
  executable: "/home/dev/.local/bin/compass",
  version: "0.3.0",
  source: "common"
};

function session(overrides: Partial<RepositorySession> = {}): RepositorySession {
  return {
    root: "/repo",
    capabilities: undefined,
    capabilityError: "not negotiated",
    activeWriter: undefined,
    watch: undefined,
    ...overrides
  } as RepositorySession;
}

describe("CompassRuntime", () => {
  it("verifies, persists, publishes, and notifies in one activation", async () => {
    const processes = new CompassProcessManager("compass");
    const repository = session();
    const persistCliPath = vi.fn(async () => {});
    const runJson = vi.fn(async () => capabilities);
    const runtime = new CompassRuntime(missing, {
      processes,
      sessions: () => [repository],
      persistCliPath,
      createCandidateProcesses: () => ({ runJson }) as never
    });
    const listener = vi.fn();
    runtime.onDidChange(listener);

    await expect(runtime.activate(installation)).resolves.toEqual({
      installation,
      capabilities
    });

    expect(runJson).toHaveBeenCalledWith(
      "/repo",
      ["capabilities", "--format", "json"],
      expect.anything()
    );
    expect(persistCliPath).toHaveBeenCalledWith(installation.executable);
    expect(processes.executablePath).toBe(installation.executable);
    expect(repository.capabilities).toBe(capabilities);
    expect(repository.capabilityError).toBeUndefined();
    expect(runtime.discovery).toMatchObject({
      kind: "found",
      executable: installation.executable,
      version: "0.3.0",
      installations: [installation]
    });
    expect(listener).toHaveBeenCalledOnce();
  });

  it("rejects incompatible capabilities without mutating runtime state", async () => {
    const processes = new CompassProcessManager("compass");
    const repository = session();
    const persistCliPath = vi.fn(async () => {});
    const runtime = new CompassRuntime(missing, {
      processes,
      sessions: () => [repository],
      persistCliPath,
      createCandidateProcesses: () => ({
        runJson: vi.fn(async () => ({
          ...capabilities,
          features: { ...capabilities.features, history: false }
        }))
      }) as never
    });

    await expect(runtime.activate(installation)).rejects.toThrow(
      "does not advertise the 'history' feature"
    );
    expect(persistCliPath).not.toHaveBeenCalled();
    expect(processes.executablePath).toBe("compass");
    expect(repository.capabilities).toBeUndefined();
    expect(runtime.discovery).toBe(missing);
  });

  it("rejects Compass versions below 0.3.0 without mutating runtime state", async () => {
    const processes = new CompassProcessManager("compass");
    const repository = session();
    const persistCliPath = vi.fn(async () => {});
    const runtime = new CompassRuntime(missing, {
      processes,
      sessions: () => [repository],
      persistCliPath,
      createCandidateProcesses: () => ({
        runJson: vi.fn(async () => ({
          ...capabilities,
          compass_version: "0.2.9"
        }))
      }) as never
    });

    await expect(runtime.activate({ ...installation, version: "0.2.9" }))
      .rejects.toThrow("requires Compass CLI 0.3.0 or newer");
    expect(persistCliPath).not.toHaveBeenCalled();
    expect(processes.executablePath).toBe("compass");
    expect(repository.capabilities).toBeUndefined();
    expect(runtime.discovery).toBe(missing);
  });

  it("rejects switching while a writer or watcher is active", async () => {
    const running = {
      operationId: "active-operation",
      completed: Promise.resolve({ code: 0, stdout: "", stderr: "" }),
      cancel: vi.fn()
    };
    for (const active of [
      { activeWriter: running },
      { watch: running }
    ]) {
      const candidate = vi.fn();
      const runtime = new CompassRuntime(missing, {
        processes: new CompassProcessManager("compass"),
        sessions: () => [session(active)],
        persistCliPath: vi.fn(async () => {}),
        createCandidateProcesses: candidate
      });

      await expect(runtime.activate(installation)).rejects.toThrow(
        "Stop active Compass builds and watchers"
      );
      expect(candidate).not.toHaveBeenCalled();
    }
  });

  it("deduplicates the activated path and fills a missing version", async () => {
    const previous: CompassDiscovery = {
      kind: "found",
      executable: "/opt/compass",
      version: "0.3.1",
      installations: [
        { ...installation, version: undefined },
        {
          executable: "/opt/compass",
          version: "0.3.1",
          source: "path"
        }
      ],
      searched: [installation.executable, "/opt/compass"]
    };
    const runtime = new CompassRuntime(previous, {
      processes: new CompassProcessManager("/opt/compass"),
      sessions: () => [],
      persistCliPath: vi.fn(async () => {}),
      createCandidateProcesses: () => ({
        runJson: vi.fn(async () => capabilities)
      }) as never
    });

    const activated = await runtime.activate({ ...installation, version: undefined });

    expect(activated.installation.version).toBe("0.3.0");
    expect(runtime.discovery.installations.map((item) => item.executable)).toEqual([
      installation.executable,
      "/opt/compass"
    ]);
  });
});
