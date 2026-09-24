// @vitest-environment jsdom

import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphEdge, GraphNode, GraphViewModel } from "../contracts/graph";
import { COMMUNITY_OVERVIEW_NODE_THRESHOLD } from "./communityOverview";
import { CompassGraph } from "./CompassGraph";

const mock = vi.hoisted(() => ({
  pendingDataSets: [] as Array<Array<Record<string, unknown>>>,
  canvases: [] as Array<{
    nodes: Array<Record<string, unknown>>;
    edges: Array<Record<string, unknown>>;
  }>,
  eventHandlers: new Map<string, Array<(parameters: unknown) => void>>()
}));

vi.mock("vis-network/standalone", () => ({
  DataSet: class {
    private readonly items = new Map<string, Record<string, unknown>>();

    constructor(items: Array<Record<string, unknown>>) {
      for (const item of items) this.items.set(String(item.id), { ...item });
      mock.pendingDataSets.push([...this.items.values()]);
    }

    update(items: Array<Record<string, unknown>>) {
      for (const item of items) {
        const id = String(item.id);
        this.items.set(id, { ...this.items.get(id), ...item });
      }
    }
  },
  Network: class {
    constructor() {
      mock.canvases.push({
        nodes: mock.pendingDataSets[0] ?? [],
        edges: mock.pendingDataSets[1] ?? []
      });
      mock.pendingDataSets.length = 0;
    }

    on(event: string, callback: (parameters: unknown) => void) {
      const handlers = mock.eventHandlers.get(event) ?? [];
      handlers.push(callback);
      mock.eventHandlers.set(event, handlers);
    }

    once(event: string, callback: (parameters: unknown) => void) {
      this.on(event, callback);
    }

    off() {}

    setOptions() {}
    stopSimulation() {}
    startSimulation() {}
    fit() {}
    destroy() {}
    getConnectedNodes() { return []; }
    getViewPosition() { return { x: 0, y: 0 }; }
    getScale() { return 1; }
    getPositions(ids: string[] = []) {
      return Object.fromEntries(ids.map((id) => [id, { x: 0, y: 0 }]));
    }
    moveNode() {}
    redraw() {}
    unselectAll() {}
    selectNodes() {}
    focus() {}
    moveTo() {}
  }
}));

function fixture(): GraphViewModel {
  const nodes: GraphNode[] = [];
  const edges: GraphEdge[] = [];
  for (let index = 0; index < COMMUNITY_OVERVIEW_NODE_THRESHOLD; index += 1) {
    const community = index % 3;
    const id = `symbol-${index}`;
    nodes.push({
      id,
      label: `symbol_${index}`,
      kind: "function",
      community,
      degree: index % 9,
      source: { file: `src/module_${community}.rs`, startLine: index + 1 }
    });
    if (index > 0 && index % 4 === 0) {
      edges.push({
        id: `edge-${index}`,
        source: `symbol-${index - 1}`,
        target: id,
        relation: "calls"
      });
    }
    if (index % 7 === 0) {
      edges.push({
        id: `cross-${index}`,
        source: id,
        target: `symbol-${(index + 1) % 3}`,
        relation: "calls"
      });
    }
  }
  return {
    schema: "compass.viewer.graph/1",
    title: "Generated fixture",
    stats: { nodes: nodes.length, edges: edges.length, communities: 3, aggregated: false },
    nodes,
    edges,
    communities: [
      { id: 0, label: "Core runtime", color: "#4E79A7", hidden: false },
      { id: 1, label: "Command line", color: "#F28E2B", hidden: false },
      { id: 2, label: "Fixtures", color: "#E15759", hidden: false }
    ],
    hyperedges: []
  };
}

function latestNodes(): Array<Record<string, unknown>> {
  return mock.canvases.at(-1)?.nodes ?? [];
}

function doubleClickNode(id: string): void {
  act(() => {
    for (const handler of mock.eventHandlers.get("doubleClick") ?? []) {
      handler({ nodes: [id], edges: [], pointer: { DOM: { x: 0, y: 0 } } });
    }
  });
}

/** Finish the one-shot arrangement a derived community detail starts with. */
function stabilize(): void {
  act(() => {
    for (const handler of mock.eventHandlers.get("stabilizationIterationsDone") ?? []) {
      handler({});
    }
  });
}

function toolbarStatus(): string {
  return document.querySelector(".compass-viewer-status-text")?.textContent ?? "";
}

afterEach(() => {
  mock.pendingDataSets.length = 0;
  mock.canvases.length = 0;
  mock.eventHandlers.clear();
  vi.unstubAllGlobals();
});

beforeEach(() => {
  vi.stubGlobal("matchMedia", vi.fn(() => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn()
  })));
});

describe("CompassGraph community overview", () => {
  it("opens a large repository on labelled community bubbles", () => {
    render(<CompassGraph model={fixture()} host={{ openSource: vi.fn() }} />);
    stabilize();

    const nodes = latestNodes();
    expect(nodes).toHaveLength(3);
    expect(nodes.map((node) => node.id)).toEqual([
      "community:0",
      "community:1",
      "community:2"
    ]);
    expect(nodes[0]).toMatchObject({
      label: expect.stringContaining("Core runtime"),
      shape: "dot"
    });
    expect(String(nodes[0]?.label)).toContain("symbols");
    expect(toolbarStatus()).toBe("3 communities · 600 symbols");
    expect(screen.getByRole("button", { name: "Communities" }))
      .toHaveAttribute("aria-pressed", "true");
  });

  it("drills into one community and returns to the overview", () => {
    render(<CompassGraph model={fixture()} host={{ openSource: vi.fn() }} />);
    stabilize();

    doubleClickNode("community:0");
    const members = latestNodes();
    expect(members.every((node) => String(node.id).startsWith("symbol-"))).toBe(true);
    expect(members.length).toBe(200);
    // A drill-down is arranged once so the symbols arrive in reading order.
    expect(screen.getByText("Arranging graph layout")).toBeInTheDocument();
    stabilize();
    expect(toolbarStatus()).toContain("Core runtime");

    fireEvent.click(screen.getByRole("button", { name: "Back to community overview" }));
    expect(latestNodes().map((node) => node.id)).toEqual([
      "community:0",
      "community:1",
      "community:2"
    ]);
  });

  it("switches between the community overview and every symbol", () => {
    render(<CompassGraph model={fixture()} host={{ openSource: vi.fn() }} />);
    stabilize();

    fireEvent.click(screen.getByRole("button", { name: "Symbols" }));
    // Every interactive view arranges itself on open, so the symbol canvas
    // arrives behind the same arranging screen.
    stabilize();
    expect(latestNodes()).toHaveLength(COMMUNITY_OVERVIEW_NODE_THRESHOLD);
    expect(screen.getByRole("button", { name: "Symbols" }))
      .toHaveAttribute("aria-pressed", "true");

    fireEvent.click(screen.getByRole("button", { name: "Communities" }));
    stabilize();
    expect(latestNodes()).toHaveLength(3);
  });

  it("returns to the overview on Escape", () => {
    render(<CompassGraph model={fixture()} host={{ openSource: vi.fn() }} />);
    stabilize();

    doubleClickNode("community:1");
    stabilize();
    expect(toolbarStatus()).toContain("Command line");

    fireEvent.keyDown(document, { key: "Escape" });
    expect(latestNodes()).toHaveLength(3);
  });

  it("keeps a small repository on the symbol canvas", () => {
    const model = fixture();
    const small: GraphViewModel = {
      ...model,
      stats: { ...model.stats, nodes: 3 },
      nodes: model.nodes.slice(0, 3)
    };
    render(<CompassGraph model={small} host={{ openSource: vi.fn() }} />);
    stabilize();

    expect(latestNodes().map((node) => node.id)).toEqual([
      "symbol-0",
      "symbol-1",
      "symbol-2"
    ]);
    expect(screen.queryByRole("button", { name: "Communities" })).toBeNull();
  });

  it("leaves a drill-down when the reader asks for every symbol", () => {
    render(<CompassGraph model={fixture()} host={{ openSource: vi.fn() }} />);
    stabilize();

    doubleClickNode("community:2");
    stabilize();
    expect(latestNodes().every((node) => String(node.id).startsWith("symbol-"))).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Symbols" }));
    expect(latestNodes()).toHaveLength(COMMUNITY_OVERVIEW_NODE_THRESHOLD);
    expect(screen.queryByRole("button", { name: "Back to community overview" })).toBeNull();
  });

  it("shows the repository path and walks back up it", () => {
    render(<CompassGraph model={fixture()} host={{ openSource: vi.fn() }} />);
    stabilize();

    const path = screen.getByRole("navigation", { name: "Graph path" });
    expect(within(path).getByText("Repository")).toHaveAttribute("aria-current", "page");
    expect(within(path).queryByRole("button")).toBeNull();

    doubleClickNode("community:1");
    stabilize();
    const detailPath = screen.getByRole("navigation", { name: "Graph path" });
    expect(within(detailPath).getByText("Command line")).toHaveAttribute("aria-current", "page");

    fireEvent.click(within(detailPath).getByRole("button", { name: "Repository" }));
    stabilize();
    expect(latestNodes()).toHaveLength(3);
    expect(within(screen.getByRole("navigation", { name: "Graph path" }))
      .getByText("Repository")).toHaveAttribute("aria-current", "page");
  });

  it("opens a community from the community list", () => {
    render(<CompassGraph model={fixture()} host={{ openSource: vi.fn() }} />);
    stabilize();

    fireEvent.click(screen.getByRole("button", { name: "Open group Core runtime" }));
    stabilize();

    expect(latestNodes().every((node) => String(node.id).startsWith("symbol-"))).toBe(true);
    expect(toolbarStatus()).toContain("Core runtime");
  });

  it("discloses a bounded drill-down in the view instead of the node limit", () => {
    const oversized = fixture();
    const community = {
      ...oversized,
      nodes: [
        ...oversized.nodes,
        ...Array.from({ length: 2_100 }, (_, index) => ({
          id: `huge-${index}`,
          label: `huge_${index}`,
          kind: "function",
          community: 0,
          degree: index % 5
        }))
      ]
    };
    render(<CompassGraph model={community} host={{ openSource: vi.fn() }} />);
    stabilize();

    doubleClickNode("community:0");
    stabilize();
    expect(latestNodes()).toHaveLength(2_000);
    expect(screen.getByText("Most connected symbols first")).toBeInTheDocument();
    expect(screen.queryByText("Partial community comparison")).toBeNull();
  });
});
