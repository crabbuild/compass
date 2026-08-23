import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => ({
  ViewColumn: { Active: 1 },
  Uri: { joinPath: vi.fn(() => ({ toString: () => "asset" })) },
  window: { createWebviewPanel: vi.fn() }
}));
vi.mock("./sourceNavigation", () => ({ openGraphSource: vi.fn() }));

import { ArchitecturePanelController, ARCHITECTURE_STDOUT_LIMIT } from "./architecturePanel";

function callflowPayload(): string {
  const sourceScopes = { production: 2, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0 };
  const group = (id: string, name: string, community: number) => ({
    id, parentId: null, kind: "subsystem", rank: community + 1,
    name: {
      value: name, provenance: "path", membershipSignature: `signature-${id}`,
      quality: 90, evidence: [`path:${name}`]
    },
    ownerKey: `crates/${id}`, communityIds: [community], nodeCount: 1,
    relationshipCount: 1, neighborCount: 1, cohesion: 0.8,
    sourceScopes: { ...sourceScopes, production: 1 }, pinned: false
  });
  const quality = {
    status: "good",
    metrics: {
      sourceScopes, unknownSourceFraction: 0, generatedVendorLeakage: 0,
      representedNodeFraction: 1, representedRelationshipFraction: 1,
      duplicateNames: 0, fallbackNames: 0, largestGroupFraction: 0.5,
      unknownRelations: 0, unassignedNodes: 0, unassignedRelationships: 0
    },
    diagnostics: []
  };
  const projection = (scope: "production" | "all_code") => ({
    scope, defaultLens: "architecture",
    groups: [group("api", "API", 0), group("storage", "Storage", 1)],
    memberships: [
      { nodeIndex: 0, groupIndex: 0 },
      { nodeIndex: 1, groupIndex: 1 }
    ],
    routes: [], overviewGroupIds: ["api", "storage"], overviewRouteIds: [],
    coverage: {
      admitted: 1, internal: 0, crossGroup: 1, unassigned: 0,
      relationClasses: { execution: 1, dependency: 0, type: 0, structure: 0, contextual: 0, unknown: 0 }
    },
    omissions: {
      totalGroups: 2, shownGroups: 2, omittedGroups: 0,
      representedNodes: 2, omittedNodes: 0,
      representedRelationships: 1, omittedRelationships: 0,
      witnessGroupIds: [], maxOverviewGroups: 24, maxOverviewRoutes: 64
    },
    quality
  });
  return JSON.stringify({
    schema: "compass.viewer.architecture/1",
    title: "Fixture — Architecture Flow",
    nodes: [
      {
        id: "handler", label: `handler-${"x".repeat(9 * 1024 * 1024)}`, kind: "function",
        sourceFile: "src/api.ts", sourceScope: "production", scopeReason: "source_path", community: 0
      },
      {
        id: "store", label: "store", kind: "function", sourceFile: "src/store.ts",
        sourceScope: "production", scopeReason: "source_path", community: 1
      }
    ],
    relationships: [{
      id: "relationship-1", source: "handler", target: "store", relation: "calls",
      relationClass: "execution", confidence: "extracted"
    }],
    projections: [projection("production"), projection("all_code")],
    statistics: { nodes: 2, relationships: 1, communities: 2, extracted: 1, inferred: 0, ambiguous: 0 },
    provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null },
    limits: {
      maxNodes: 250000, maxRelationships: 1000000, maxGroups: 100000, maxRoutes: 250000,
      maxOverviewGroups: 24, maxOverviewRoutes: 64, maxNameCandidates: 12,
      maxNameEvidence: 4, maxDiagnostics: 128, maxOmissionWitnesses: 8
    },
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
    expect(overview.model.statistics).toMatchObject({ totalNodes: 2, totalRelationships: 1 });
    expect(overview.model).not.toHaveProperty("crossSectionCalls");
    expect(JSON.stringify(overview).length).toBeLessThan(100_000);
  });
});
