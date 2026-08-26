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
  it("shows directional relationships and exposes icon-led node actions", () => {
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

    expect(screen.getByText("Incoming", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByText("Outgoing", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByRole("button", {
      name: "Focus Caller; incoming; calls"
    })).toBeInTheDocument();
    expect(screen.getByRole("button", {
      name: "Focus Callee; outgoing; calls"
    })).toBeInTheDocument();

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
});
