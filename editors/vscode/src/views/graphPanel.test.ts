import { describe, expect, it, vi } from "vitest";

const receiveMessage = vi.fn();
const postMessage = vi.fn(async (_message: unknown) => true);

vi.mock("vscode", () => ({
  ViewColumn: { Active: 1 },
  Uri: { joinPath: vi.fn(() => ({ toString: () => "asset" })) },
  commands: { executeCommand: vi.fn() },
  window: {
    createWebviewPanel: vi.fn(() => ({
      webview: {
        asWebviewUri: vi.fn(() => ({ toString: () => "asset" })),
        cspSource: "test-source",
        onDidReceiveMessage: receiveMessage,
        postMessage
      },
      onDidDispose: vi.fn()
    })),
    showErrorMessage: vi.fn(),
    showWarningMessage: vi.fn()
  },
  workspace: {
    asRelativePath: vi.fn((value: string) => value),
    getConfiguration: vi.fn(() => ({ get: vi.fn(() => 5000) }))
  }
}));
vi.mock("./sourceNavigation", () => ({ openGraphSource: vi.fn() }));
vi.mock("./graphOverview", () => ({
  graphOverviewCachePath: vi.fn(() => "/cache/overview.json"),
  graphSourceInfo: vi.fn(async () => ({ bytes: 9 * 1024 * 1024 })),
  loadCachedGraphOverview: vi.fn(async () => undefined),
  loadPreparedGraphOverview: vi.fn(async () => undefined),
  writeCachedGraphOverview: vi.fn(async () => undefined)
}));
vi.mock("./graphSnapshot", () => ({
  CurrentGraphSnapshot: class {
    replace = vi.fn(async () => "/snapshot/graph.json");
    dispose = vi.fn(async () => undefined);
  }
}));

import { GRAPH_EXPORT_STDOUT_LIMIT, GraphPanel } from "./graphPanel";

const model = {
  schema: "compass.viewer.graph/1",
  title: "graph.json",
  stats: { nodes: 1, edges: 0, communities: 1, aggregated: false },
  nodes: [{ id: "a", label: "A", community: 0 }],
  edges: [],
  communities: [{ id: 0, label: "Community 0", color: "#4E79A7", hidden: false }],
  hyperedges: []
};

describe("GraphPanel", () => {
  it("allows graph overview stdout up to 256 MiB", async () => {
    const runJson = vi.fn(async () => model);
    const session = {
      id: "/repo",
      root: "/repo",
      graphPath: "/repo/compass-out/graph.json",
      processes: { runJson }
    };
    const context = {
      extensionUri: {},
      storageUri: { fsPath: "/storage" },
      globalStorageUri: { fsPath: "/global-storage" }
    };

    await GraphPanel.open(
      context as never,
      session as never,
      { appendLine: vi.fn() } as never
    );
    const handler = receiveMessage.mock.calls[0]?.[0] as
      | ((message: unknown) => Promise<void>)
      | undefined;
    if (!handler) throw new Error("Expected graph webview message handler");
    await handler({ type: "ready" });

    expect(runJson).toHaveBeenCalledWith(
      "/repo",
      [
        "export", "json", "--graph", "/snapshot/graph.json",
        "--node-limit", "5000"
      ],
      expect.anything(),
      expect.any(AbortSignal),
      { stdoutBytes: GRAPH_EXPORT_STDOUT_LIMIT }
    );
    expect(GRAPH_EXPORT_STDOUT_LIMIT).toBe(256 * 1024 * 1024);
  });
});
