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
    stageOverlay
  }: {
    model: GraphViewModel;
    communityDetail?: { communityId: number; model: GraphViewModel };
    host: { openCommunity?(communityId: number): void };
    toolbarLeading?: ReactNode;
    toolbarLeadingPanel?: ReactNode;
    stageOverlay?: ReactNode;
  }) {
    const active = communityDetail?.model ?? model;
    return (
      <div>
        {toolbarLeading}
        {toolbarLeadingPanel}
        {stageOverlay}
        <output data-testid="visible-nodes">{active.nodes.length}</output>
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
});
