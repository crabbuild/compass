import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { GraphInspector } from "./GraphInspector";

const model: GraphViewModel = {
  schema: "compass.viewer.graph/1",
  title: "Inspector fixture",
  stats: { nodes: 3, edges: 2, communities: 1, aggregated: false },
  nodes: [
    { id: "caller", label: "Caller", kind: "function", community: 0 },
    {
      id: "selected",
      label: "Selected",
      kind: "function",
      community: 0,
      language: "rust",
      source: { file: "src/lib.rs", startLine: 4, endLine: 7 }
    },
    { id: "callee", label: "Callee", kind: "function", community: 0 }
  ],
  edges: [
    { id: "incoming", source: "caller", target: "selected", relation: "calls" },
    { id: "outgoing", source: "selected", target: "callee", relation: "calls" }
  ],
  communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
  hyperedges: []
};

describe("GraphInspector selected node details", () => {
  it("switches directional relationships with accessible tabs and exposes node actions", () => {
    const onFocus = vi.fn();
    const onQueryNode = vi.fn();
    const selected = model.nodes[1];
    if (!selected) throw new Error("fixture selected node is missing");

    render(
      <GraphInspector
        model={model}
        selected={selected}
        neighbors={[model.nodes[0], model.nodes[2]].filter((node) => node !== undefined)}
        connectedEdges={model.edges}
        query=""
        matches={[]}
        hiddenCommunities={new Set()}
        comparisonMode={false}
        renderedEdgeCount={2}
        showHeader={false}
        onQueryChange={vi.fn()}
        onFocus={onFocus}
        onOpenSource={vi.fn()}
        onQueryNode={onQueryNode}
        onToggleCommunity={vi.fn()}
        onSetAllVisible={vi.fn()}
        collapsed={false}
        onToggleCollapsed={vi.fn()}
      />
    );

    const incoming = screen.getByRole("tab", { name: /Incoming/ });
    const outgoing = screen.getByRole("tab", { name: /Outgoing/ });
    expect(document.getElementById(incoming.getAttribute("aria-controls") ?? ""))
      .toHaveAttribute("hidden");
    expect(outgoing).toHaveAttribute("aria-selected", "true");
    expect(incoming).toHaveAttribute("aria-selected", "false");
    expect(screen.queryByRole("button", { name: "Focus Caller; incoming; calls" }))
      .toBeNull();
    expect(screen.getByRole("button", {
      name: "Focus Callee; outgoing; calls"
    })).toBeInTheDocument();

    fireEvent.click(incoming);
    expect(incoming).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("button", { name: "Focus Caller; incoming; calls" }))
      .toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Focus Callee; outgoing; calls" }))
      .toBeNull();
    fireEvent.keyDown(incoming, { key: "ArrowRight" });
    expect(outgoing).toHaveAttribute("aria-selected", "true");
    expect(outgoing).toHaveFocus();
    fireEvent.keyDown(outgoing, { key: "Home" });
    expect(incoming).toHaveAttribute("aria-selected", "true");
    expect(incoming).toHaveFocus();

    const actions = screen.getByLabelText("Code graph queries");
    for (const action of ["Callers", "Callees", "Impact"]) {
      expect(within(actions).getByRole("button", { name: action })).toHaveAttribute("title");
    }
    fireEvent.click(within(actions).getByRole("button", { name: "Impact" }));
    expect(onQueryNode).toHaveBeenCalledWith("impact", "selected");

    fireEvent.click(screen.getByRole("button", {
      name: "Focus Caller; incoming; calls"
    }));
    expect(onFocus).toHaveBeenCalledWith("caller");
  });

  it("opens a published child subgraph and hides the action when it is unavailable", () => {
    const onOpenCommunity = vi.fn();
    const selected = {
      id: "group:0", label: "Core", kind: "community", community: 0,
      memberCount: 42, detailAvailable: false
    };
    const props = {
      model: { ...model, stats: { ...model.stats, aggregated: true } },
      selected,
      communityFacts: {
        label: "Core", symbols: 42, groupId: "group:0", level: 0,
        childGroups: 2, boundaryKinds: [], couplings: [], couplingCount: 0,
        couplingsComplete: true
      },
      neighbors: [], connectedEdges: [], query: "", matches: [],
      hiddenCommunities: new Set<number>(), comparisonMode: false,
      renderedEdgeCount: 0, showHeader: false, onQueryChange: vi.fn(),
      onFocus: vi.fn(), onOpenSource: vi.fn(), onOpenCommunity,
      onToggleCommunity: vi.fn(), onSetAllVisible: vi.fn(), collapsed: false,
      onToggleCollapsed: vi.fn()
    };
    const view = render(<GraphInspector {...props} subgraphAvailable />);
    fireEvent.click(screen.getByRole("button", { name: /Open subgraph/ }));
    expect(onOpenCommunity).toHaveBeenCalledWith(0);
    view.rerender(<GraphInspector {...props} subgraphAvailable={false} />);
    expect(screen.queryByRole("button", { name: /Open subgraph/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Open community/ })).toBeNull();
  });
});
