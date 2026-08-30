// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import type { WorkbenchModel } from "../contracts/workbench";
import { VisualizationWorkbench } from "./VisualizationWorkbench";

vi.mock("../graph/CompassGraph", () => ({
  CompassGraph({
    model,
    communityDetail,
    host,
    toolbarLeading,
    toolbarLeadingPanel,
    stageOverlay,
    preferredLayout
  }: {
    model: GraphViewModel;
    communityDetail?: { communityId: number; model: GraphViewModel };
    host: { openCommunity?(communityId: number): void };
    toolbarLeading?: ReactNode;
    toolbarLeadingPanel?: ReactNode;
    stageOverlay?: ReactNode;
    preferredLayout?: string;
  }) {
    const active = communityDetail?.model ?? model;
    return (
      <div>
        {toolbarLeading}
        {toolbarLeadingPanel}
        {stageOverlay}
        <output data-testid="visible-nodes">{active.nodes.length}</output>
        <output data-testid="preferred-layout">{preferredLayout}</output>
        <button type="button" onClick={() => host.openCommunity?.(1)}>
          Open community fixture
        </button>
      </div>
    );
  }
}));

function graph(nodes: GraphViewModel["nodes"]): GraphViewModel {
  return {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes: nodes.length, edges: 0, communities: 1, aggregated: false },
    nodes,
    edges: [],
    communities: [{ id: 1, label: "Core", color: "#4e79a7", hidden: false }],
    hyperedges: []
  };
}

describe("VisualizationWorkbench graph filters", () => {
  afterEach(cleanup);

  it("follows the visible graph when community detail opens", async () => {
    const overview = graph([
      { id: "root", label: "Root", kind: "module", community: 1 },
      { id: "helper", label: "Helper", kind: "function", community: 1 },
      { id: "readme", label: "README", kind: "document", community: 1 }
    ]);
    const detail = graph([
      { id: "helper", label: "Helper", kind: "function", community: 1 },
      { id: "store", label: "Store", kind: "type", community: 1 }
    ]);
    const workbench: WorkbenchModel = {
      schema: "compass.viewer.workbench/1",
      title: "Fixture workbench",
      graphIdentity: "fixture-identity",
      defaultView: "code",
      views: [{
        id: "code",
        kind: "code",
        title: "Code graph",
        description: "Fixture graph",
        coverage: {
          status: "complete",
          truncated: false,
          nodes: 3,
          edges: 0,
          limitations: []
        },
        model: overview,
        communityDetails: { "1": detail }
      }]
    };

    render(<VisualizationWorkbench
      workbench={workbench}
      host={{ openSource: vi.fn() }}
    />);

    expect(screen.getByLabelText("Compass navigation")).toHaveTextContent("Fixture workbench");
    fireEvent.click(screen.getByRole("button", { name: "Collapse graph navigation" }));
    expect(screen.getByLabelText("Compass navigation")).toHaveAttribute("data-collapsed", "true");
    expect(screen.getByRole("button", { name: "Expand graph navigation" })).toBeInTheDocument();

    expect(screen.getByLabelText("Graph filters")).toHaveTextContent("3 / 3");
    fireEvent.click(screen.getByRole("button", { name: "Open community fixture" }));

    await waitFor(() => {
      expect(screen.getByLabelText("Graph filters")).toHaveTextContent("2 / 2");
    });
    fireEvent.click(screen.getByLabelText("Graph filters"));
    const kind = screen.getByRole("combobox", { name: "Node kind" });
    expect(kind).not.toHaveTextContent("Module");
    fireEvent.change(kind, { target: { value: "type" } });

    expect(screen.getByTestId("visible-nodes")).toHaveTextContent("1");
    expect(screen.getByLabelText("Graph filters")).toHaveTextContent("1 / 2");
  });

  it("uses a hierarchy for routes while keeping other artifacts on the grid", () => {
    const model = graph([
      { id: "route", label: "GET /", kind: "route", community: 1 },
      { id: "handler", label: "handler()", kind: "function", community: 1 }
    ]);
    const workbench: WorkbenchModel = {
      schema: "compass.viewer.workbench/1",
      title: "Fixture workbench",
      graphIdentity: "fixture-identity",
      defaultView: "routes",
      views: [{
        id: "routes",
        kind: "artifact",
        lens: "routes",
        title: "Routes and handlers",
        description: "Route fixture",
        relations: ["routes_to"],
        coverage: {
          status: "complete",
          truncated: false,
          nodes: 2,
          edges: 1,
          limitations: []
        },
        model
      }, {
        id: "dependencies",
        kind: "artifact",
        lens: "dependencies",
        title: "Dependencies",
        description: "Dependency fixture",
        relations: ["depends_on"],
        coverage: {
          status: "complete",
          truncated: false,
          nodes: 2,
          edges: 1,
          limitations: []
        },
        model
      }]
    };

    render(<VisualizationWorkbench workbench={workbench} host={{ openSource: vi.fn() }} />);
    expect(screen.getByTestId("preferred-layout")).toHaveTextContent("hierarchical");

    fireEvent.click(screen.getByRole("button", { name: /Dependencies/ }));
    expect(screen.getByTestId("preferred-layout")).toHaveTextContent("grid");
  });

  it("closes the filter panel with Escape and restores trigger focus", () => {
    const overview = graph([
      { id: "root", label: "Root", kind: "module", community: 1 }
    ]);
    const workbench: WorkbenchModel = {
      schema: "compass.viewer.workbench/1",
      title: "Fixture workbench",
      graphIdentity: "fixture-identity",
      defaultView: "code",
      views: [{
        id: "code",
        kind: "code",
        title: "Code graph",
        description: "Fixture graph",
        coverage: {
          status: "complete",
          truncated: false,
          nodes: 1,
          edges: 0,
          limitations: []
        },
        model: overview,
        communityDetails: {}
      }]
    };

    render(<VisualizationWorkbench workbench={workbench} host={{ openSource: vi.fn() }} />);
    const trigger = screen.getByRole("button", { name: "Graph filters" });
    fireEvent.click(trigger);
    expect(screen.getByRole("region", { name: "Graph filter options" })).toBeVisible();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("region", { name: "Graph filter options" })).toBeNull();
    expect(trigger).toHaveFocus();
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("explains an empty filtered graph and clears every filter", () => {
    const overview = graph([
      { id: "root", label: "Root", kind: "module", language: "rust", community: 1 },
      { id: "helper", label: "Helper", kind: "function", language: "typescript", community: 1 }
    ]);
    const workbench: WorkbenchModel = {
      schema: "compass.viewer.workbench/1",
      title: "Fixture workbench",
      graphIdentity: "fixture-identity",
      defaultView: "code",
      views: [{
        id: "code",
        kind: "code",
        title: "Code graph",
        description: "Fixture graph",
        coverage: {
          status: "complete",
          truncated: false,
          nodes: 2,
          edges: 0,
          limitations: []
        },
        model: overview,
        communityDetails: {}
      }]
    };

    render(<VisualizationWorkbench workbench={workbench} host={{ openSource: vi.fn() }} />);
    fireEvent.click(screen.getByRole("button", { name: "Graph filters" }));
    fireEvent.change(screen.getByRole("combobox", { name: "Node kind" }), {
      target: { value: "module" }
    });
    fireEvent.change(screen.getByRole("combobox", { name: "Language" }), {
      target: { value: "typescript" }
    });

    expect(screen.getByText("No nodes match these filters")).toBeVisible();
    expect(screen.getByTestId("visible-nodes")).toHaveTextContent("0");
    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(screen.getByTestId("visible-nodes")).toHaveTextContent("2");
    expect(screen.queryByText("No nodes match these filters")).toBeNull();
  });

  it("separates extraction completeness from architecture quality and exposes omitted groups", () => {
    const counts = {
      production: 2, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0
    };
    const groups = ["api", "storage"].map((id, index) => ({
      id, parentId: null, kind: "subsystem" as const, rank: index + 1,
      name: {
        value: id === "api" ? "API" : "Storage", provenance: "path" as const,
        membershipSignature: `signature-${id}`, quality: 90, evidence: [`path:${id}`]
      },
      ownerKey: `crates/${id}`, communityIds: [index], nodeCount: 1,
      relationshipCount: 0, neighborCount: 0, cohesion: 1,
      sourceScopes: { ...counts, production: 1 }, pinned: false
    }));
    const workbench: WorkbenchModel = {
      schema: "compass.viewer.workbench/1", title: "Fixture workbench",
      graphIdentity: "fixture-identity", defaultView: "architecture",
      views: [{
        id: "architecture", kind: "architecture", title: "Architecture",
        description: "Fixture architecture",
        coverage: { status: "complete", truncated: false, nodes: 2, edges: 0, limitations: [] },
        model: {
          schema: "compass.viewer.architecture/1", title: "Fixture architecture",
          nodes: [
            { id: "a", label: "Handler", kind: "function", sourceFile: "src/a.ts", sourceScope: "production", scopeReason: "source_path", community: 0 },
            { id: "b", label: "Store", kind: "struct", sourceFile: "src/b.ts", sourceScope: "production", scopeReason: "source_path", community: 1 }
          ],
          relationships: [],
          projections: [{
            scope: "production", defaultLens: "architecture", groups,
            memberships: [{ nodeIndex: 0, groupIndex: 0 }, { nodeIndex: 1, groupIndex: 1 }],
            routes: [], overviewGroupIds: ["api"], overviewRouteIds: [],
            coverage: {
              admitted: 0, internal: 0, crossGroup: 0, unassigned: 0,
              relationClasses: { execution: 0, dependency: 0, type: 0, structure: 0, contextual: 0, unknown: 0 }
            },
            omissions: {
              totalGroups: 2, shownGroups: 1, omittedGroups: 1,
              representedNodes: 1, omittedNodes: 1,
              representedRelationships: 0, omittedRelationships: 0,
              witnessGroupIds: ["storage"], maxOverviewGroups: 1, maxOverviewRoutes: 1
            },
            quality: {
              status: "good", metrics: {
                sourceScopes: counts, unknownSourceFraction: 0, generatedVendorLeakage: 0,
                representedNodeFraction: 0.5, representedRelationshipFraction: 1,
                duplicateNames: 0, fallbackNames: 0, largestGroupFraction: 0.5,
                unknownRelations: 0, unassignedNodes: 0, unassignedRelationships: 0
              }, diagnostics: []
            }
          }],
          statistics: { nodes: 2, relationships: 0, communities: 2, extracted: 0, inferred: 0, ambiguous: 0 },
          provenance: { projectName: "Fixture", builtAtCommit: null, generatedAt: null },
          limits: {
            maxNodes: 250000, maxRelationships: 1000000, maxGroups: 100000,
            maxRoutes: 250000, maxOverviewGroups: 1, maxOverviewRoutes: 1,
            maxNameCandidates: 12, maxNameEvidence: 4, maxDiagnostics: 128,
            maxOmissionWitnesses: 8
          }
        }
      }]
    };

    render(<VisualizationWorkbench workbench={workbench} host={{ openSource: vi.fn() }} />);
    expect(screen.getByText("Extraction complete")).toBeVisible();
    expect(screen.getByText("Architecture quality: good")).toBeVisible();
    expect(screen.getByText(/1 of 2 groups shown · 1 available in directory/)).toBeVisible();
    fireEvent.change(screen.getByRole("combobox", { name: "Subsystem directory" }), {
      target: { value: "storage" }
    });
    expect(screen.getByRole("heading", { name: "Storage" })).toBeVisible();
  });
});
