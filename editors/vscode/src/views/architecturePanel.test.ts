import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => ({
  ViewColumn: { Active: 1 },
  Uri: { joinPath: vi.fn(() => ({ toString: () => "asset" })) },
  window: { createWebviewPanel: vi.fn() }
}));
vi.mock("./sourceNavigation", () => ({ openGraphSource: vi.fn() }));

import { ArchitecturePanelController, ARCHITECTURE_STDOUT_LIMIT } from "./architecturePanel";

function callflowPayload(): string {
  return JSON.stringify({
    schema: "compass.viewer.callflow/2",
    title: "Fixture — Architecture Flow",
    sections: [
      {
        id: "overview", name: "Architecture Overview", communities: [],
        nodeCount: 0, internalCallCount: 0, nodes: [], edges: []
      },
      {
        id: "api", name: "API", communities: ["0"],
        nodeCount: 1, internalCallCount: 0,
        nodes: [{
          id: "handler", label: "handler", kind: "function",
          sourceFile: "src/api.ts", scope: "production"
        }],
        edges: []
      },
      {
        id: "storage", name: "Storage", communities: ["1"],
        nodeCount: 1, internalCallCount: 0,
        nodes: [{
          id: "store", label: "store", kind: "function",
          sourceFile: "src/store.ts", scope: "production"
        }],
        edges: []
      }
    ],
    overviewLinks: [{ sourceSection: "api", targetSection: "storage", calls: 1 }],
    crossSectionCalls: [{
      source: "handler", target: "store", sourceSection: "api",
      targetSection: "storage", relation: "calls", confidence: "extracted"
    }],
    coverage: { internal: 0, crossSection: 1, unassigned: 0 },
    reportHighlights: ["x".repeat(9 * 1024 * 1024)],
    statistics: {
      nodes: 2, edges: 1, communities: 2, hyperedges: 0,
      extracted: 1, inferred: 0, ambiguous: 0
    },
    provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null }
  });
}

describe("ArchitecturePanelController", () => {
  it("captures large exports in the host and posts only a bounded overview", async () => {
    const run = vi.fn(async () => ({ code: 0, stdout: callflowPayload(), stderr: "" }));
    const postMessage = vi.fn(async (_message: unknown) => true);
    const panel = {
      webview: { postMessage },
      onDidDispose: vi.fn()
    };
    const output = { appendLine: vi.fn(), show: vi.fn() };
    const session = {
      id: "/repo",
      root: "/repo",
      graphPath: "/repo/compass-out/graph.json",
      processes: { run }
    };
    const controller = new ArchitecturePanelController(
      {} as never,
      session as never,
      panel as never,
      output as never
    );

    await controller.handleMessage({ type: "ready" });

    expect(run).toHaveBeenCalledWith(
      "/repo",
      ["export", "callflow-json", "--graph", "/repo/compass-out/graph.json"],
      expect.any(AbortSignal),
      { stdoutBytes: ARCHITECTURE_STDOUT_LIMIT }
    );
    const overview = postMessage.mock.calls
      .map(([message]) => message as { type?: string; model?: Record<string, unknown> })
      .find((message) => message.type === "architectureOverview");
    expect(overview).toBeDefined();
    if (!overview?.model) throw new Error("Expected architecture overview");
    expect(overview.model.statistics).toMatchObject({ totalNodes: 2, totalCalls: 1 });
    expect(overview.model).not.toHaveProperty("crossSectionCalls");
    expect(JSON.stringify(overview).length).toBeLessThan(100_000);
  });
});
