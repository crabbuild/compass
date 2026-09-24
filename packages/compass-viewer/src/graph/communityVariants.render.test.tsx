// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";
import { communityOverviewModel } from "./communityOverview";
import { communityVariantData } from "./communityVariants";
import { CommunityLanes } from "./CommunityLanes";
import { CommunityMatrix } from "./CommunityMatrix";
import { CommunityTreemap } from "./CommunityTreemap";

beforeEach(() => {
  vi.stubGlobal("matchMedia", vi.fn(() => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn()
  })));
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function node(id: string, community: number, extra: Partial<GraphNode> = {}): GraphNode {
  return {
    id,
    label: id,
    kind: "function",
    community,
    degree: 3,
    source: { file: `src/${community}.ts`, startLine: 1 },
    ...extra
  };
}

function variantFixture() {
  const nodes: GraphNode[] = [
    node("core-hub", 1, { communityName: "Core", degree: 12 }),
    node("core-a", 1),
    node("core-b", 1),
    node("ui-hub", 2, { communityName: "Interface", degree: 6 }),
    node("ui-a", 2),
    node("job-runner", 3, { communityName: "Jobs", kind: "job" })
  ];
  const edges: GraphEdge[] = [
    { id: "e1", source: "core-hub", target: "core-a", relation: "calls" },
    { id: "e2", source: "core-hub", target: "ui-hub", relation: "calls" },
    { id: "e3", source: "core-a", target: "ui-a", relation: "imports" },
    { id: "e4", source: "ui-hub", target: "job-runner", relation: "triggers" }
  ];
  const model: GraphViewModel = {
    schema: "compass.viewer.graph/1",
    title: "Variant fixture",
    stats: { nodes: nodes.length, edges: edges.length, communities: 3, aggregated: false },
    nodes,
    edges,
    communities: [
      { id: 1, label: "Core", color: "#4E79A7", hidden: false },
      { id: 2, label: "Interface", color: "#F28E2B", hidden: false },
      { id: 3, label: "Jobs", color: "#E15759", hidden: false }
    ],
    hyperedges: []
  };
  const overview = communityOverviewModel(model);
  const data = communityVariantData(overview.model, overview);
  if (!data) throw new Error("variant data missing");
  return data;
}

describe("CommunityMatrix", () => {
  it("renders a labelled grid, keeps the diagonal inert, and opens a target community", () => {
    const data = variantFixture();
    const onOpenCommunity = vi.fn();
    render(<CommunityMatrix data={data} onOpenCommunity={onOpenCommunity} />);

    expect(screen.getByRole("grid", { name: "Community coupling matrix" }))
      .toBeInTheDocument();
    expect(screen.getAllByRole("row")).toHaveLength(data.communities.length + 1);
    expect(screen.getByRole("button", { name: /Core to Interface: 1 calls · 1 imports/ }))
      .toBeEnabled();
    expect(screen.getByRole("button", { name: "Core, 3 symbols" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: /Core to Interface/ }));
    expect(onOpenCommunity).toHaveBeenCalledWith(2);
  });

  it("says when the export carries no relationship kinds", () => {
    const data = variantFixture();
    const untyped = {
      ...data,
      links: data.links.map((link) => ({
        ...link,
        relation: `${link.weight} cross-community edges`,
        category: "other" as const
      }))
    };
    render(<CommunityMatrix data={untyped} onOpenCommunity={vi.fn()} />);
    expect(screen.getByText(/records relationship counts without kinds/))
      .toBeInTheDocument();
  });
});

describe("CommunityTreemap", () => {
  it("labels tiles by symbol area and opens a community", () => {
    const data = variantFixture();
    const onOpenCommunity = vi.fn();
    render(<CommunityTreemap data={data} onOpenCommunity={onOpenCommunity} />);

    expect(screen.getByRole("group", { name: "Community area map" }))
      .toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Core, 3 symbols" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Jobs, 1 symbols" }));
    expect(onOpenCommunity).toHaveBeenCalledWith(3);
  });
});

describe("CommunityLanes", () => {
  it("draws tier bars and coupling ribbons", () => {
    const data = variantFixture();
    const onOpenCommunity = vi.fn();
    const { container } = render(
      <CommunityLanes data={data} onOpenCommunity={onOpenCommunity} />
    );

    expect(screen.getByRole("group", { name: "Community tier map" }))
      .toBeInTheDocument();
    expect(container.querySelectorAll(".compass-lane-tier")).toHaveLength(1);
    expect(container.querySelectorAll(".compass-lane-ribbon").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Interface, 2 symbols" }));
    expect(onOpenCommunity).toHaveBeenCalledWith(2);
  });
});
