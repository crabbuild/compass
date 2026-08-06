// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { VisNetworkCanvas } from "./VisNetworkCanvas";

const mock = vi.hoisted(() => ({
  dataSets: [] as Array<Array<Record<string, unknown>>>,
  networks: 0
}));

vi.mock("vis-network/standalone", () => ({
  DataSet: class {
    private readonly items = new Map<string, Record<string, unknown>>();

    constructor(items: Array<Record<string, unknown>>) {
      for (const item of items) this.items.set(String(item.id), { ...item });
      mock.dataSets.push([...this.items.values()]);
    }

    update(items: Array<Record<string, unknown>>) {
      for (const item of items) {
        const id = String(item.id);
        this.items.set(id, { ...this.items.get(id), ...item });
      }
    }

    get(id: string) {
      return this.items.get(id);
    }
  },
  Network: class {
    constructor() {
      mock.networks += 1;
    }

    setOptions() {}
    stopSimulation() {}
    startSimulation() {}
    fit() {}
    destroy() {}
    on() {}
    once() {}
    getConnectedNodes() { return []; }
    getViewPosition() { return { x: 0, y: 0 }; }
    getScale() { return 1; }
    unselectAll() {}
    selectNodes() {}
    focus() {}
  }
}));

const model: GraphViewModel = {
  schema: "compass.viewer.graph/1",
  title: "Fixture",
  stats: { nodes: 2, edges: 1, communities: 1, aggregated: false },
  nodes: [
    { id: "caller", label: "caller", community: 0 },
    { id: "callee", label: "callee", community: 0 }
  ],
  edges: [{
    id: "caller-callee",
    source: "caller",
    target: "callee",
    relation: "calls",
    confidence: "inferred"
  }],
  communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
  hyperedges: []
};

describe("VisNetworkCanvas hover lifecycle", () => {
  beforeEach(() => {
    mock.dataSets.length = 0;
    mock.networks = 0;
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

  it("does not create the native vis-network edge tooltip", () => {
    render(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={false}
      layoutStyle="automatic"
      forceLabels={false}
      hiddenCommunities={new Set()}
      hiddenChanges={new Set()}
      onFocus={vi.fn()}
      onOpenSource={vi.fn()}
      onOpenRelationshipSource={vi.fn()}
      onHover={vi.fn()}
      onHoverEdge={vi.fn()}
      onClear={vi.fn()}
      onStabilized={vi.fn()}
    />);

    expect(mock.dataSets[1]?.[0]).not.toHaveProperty("title");
  });

  it("clears transient hover when the pointer leaves the graph region", () => {
    const onHover = vi.fn();
    const onHoverEdge = vi.fn();
    render(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={false}
      layoutStyle="automatic"
      forceLabels={false}
      hiddenCommunities={new Set()}
      hiddenChanges={new Set()}
      onFocus={vi.fn()}
      onOpenSource={vi.fn()}
      onOpenRelationshipSource={vi.fn()}
      onHover={onHover}
      onHoverEdge={onHoverEdge}
      onClear={vi.fn()}
      onStabilized={vi.fn()}
    />);

    fireEvent.mouseLeave(screen.getByRole("region", {
      name: "Interactive Compass code graph"
    }));
    expect(onHover).toHaveBeenCalledWith(null);
    expect(onHoverEdge).toHaveBeenCalledWith(null);
  });

  it("does not rebuild the network when event callback identities change", () => {
    const firstCallbacks = {
      onFocus: vi.fn(),
      onOpenSource: vi.fn(),
      onOpenRelationshipSource: vi.fn(),
      onHover: vi.fn(),
      onHoverEdge: vi.fn(),
      onClear: vi.fn(),
      onStabilized: vi.fn()
    };
    const { rerender } = render(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={false}
      layoutStyle="automatic"
      forceLabels={false}
      hiddenCommunities={new Set()}
      hiddenChanges={new Set()}
      {...firstCallbacks}
    />);

    rerender(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={false}
      layoutStyle="automatic"
      forceLabels={false}
      hiddenCommunities={new Set()}
      hiddenChanges={new Set()}
      onFocus={vi.fn()}
      onOpenSource={vi.fn()}
      onOpenRelationshipSource={vi.fn()}
      onHover={vi.fn()}
      onHoverEdge={vi.fn()}
      onClear={vi.fn()}
      onStabilized={vi.fn()}
    />);

    expect(mock.networks).toBe(1);
  });

  it("seeds explicit positions for a paused graph", () => {
    render(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={false}
      layoutStyle="automatic"
      initialPositions={new Map([
        ["caller", { x: -40, y: 12 }],
        ["callee", { x: 48, y: 12 }]
      ])}
      forceLabels={false}
      hiddenCommunities={new Set()}
      hiddenChanges={new Set()}
      onFocus={vi.fn()}
      onOpenSource={vi.fn()}
      onOpenRelationshipSource={vi.fn()}
      onHover={vi.fn()}
      onHoverEdge={vi.fn()}
      onClear={vi.fn()}
      onStabilized={vi.fn()}
    />);

    expect(mock.dataSets[0]?.[0]).toMatchObject({
      id: "caller",
      x: -40,
      y: 12
    });
  });
});
