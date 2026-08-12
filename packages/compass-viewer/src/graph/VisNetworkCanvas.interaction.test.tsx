// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { VisNetworkCanvas, type GraphCanvasHandle } from "./VisNetworkCanvas";

const mock = vi.hoisted(() => ({
  dataSets: [] as Array<Array<Record<string, unknown>>>,
  networks: 0,
  fits: [] as Array<Record<string, unknown> | undefined>,
  moves: [] as Array<Record<string, unknown>>,
  connectedNodeRequests: [] as string[],
  updates: [] as Array<Array<Record<string, unknown>>>,
  movedNodes: [] as Array<{ id: string; x: number; y: number }>,
  simulationStarts: 0,
  simulationStops: 0
}));

vi.mock("vis-network/standalone", () => ({
  DataSet: class {
    private readonly items = new Map<string, Record<string, unknown>>();

    constructor(items: Array<Record<string, unknown>>) {
      for (const item of items) this.items.set(String(item.id), { ...item });
      mock.dataSets.push([...this.items.values()]);
    }

    update(items: Array<Record<string, unknown>>) {
      mock.updates.push(items);
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
    stopSimulation() { mock.simulationStops += 1; }
    startSimulation() { mock.simulationStarts += 1; }
    fit(options?: Record<string, unknown>) { mock.fits.push(options); }
    destroy() {}
    on() {}
    once() {}
    getConnectedNodes(id: string) {
      mock.connectedNodeRequests.push(id);
      return id === "caller" ? ["callee"] : [];
    }
    getViewPosition() { return { x: 0, y: 0 }; }
    getScale() { return 1; }
    getPositions(ids: string[] = ["caller", "callee"]) {
      return Object.fromEntries(ids.map((id, index) => [id, { x: index * 10, y: 0 }]));
    }
    moveNode(id: string, x: number, y: number) {
      mock.movedNodes.push({ id, x, y });
    }
    redraw() {}
    unselectAll() {}
    selectNodes() {}
    focus() {}
    moveTo(options: Record<string, unknown>) { mock.moves.push(options); }
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
    mock.fits.length = 0;
    mock.moves.length = 0;
    mock.connectedNodeRequests.length = 0;
    mock.updates.length = 0;
    mock.movedNodes.length = 0;
    mock.simulationStarts = 0;
    mock.simulationStops = 0;
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

  it("reheats a settled nested layout when physics resumes", () => {
    const callbacks = {
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
      {...callbacks}
    />);

    mock.movedNodes.length = 0;
    const startsBeforeResume = mock.simulationStarts;
    rerender(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={true}
      layoutStyle="automatic"
      forceLabels={false}
      hiddenCommunities={new Set()}
      hiddenChanges={new Set()}
      {...callbacks}
    />);

    expect(mock.movedNodes.map(({ id }) => id)).toEqual(["callee", "caller"]);
    expect(mock.simulationStarts).toBeGreaterThan(startsBeforeResume);
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

  it("exposes bounded zoom and selected-neighborhood camera controls", () => {
    const ref = createRef<GraphCanvasHandle>();
    render(<VisNetworkCanvas
      ref={ref}
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

    ref.current?.zoomOut();
    ref.current?.resetZoom();
    ref.current?.zoomIn();
    ref.current?.fitSelection(["caller", "callee"]);

    expect(mock.moves.map((move) => move.scale)).toEqual([0.8, 1, 1.25]);
    expect(mock.fits.at(-1)?.nodes).toEqual(["caller", "callee"]);
  });

  it("hides nodes and edges outside an isolated directed neighborhood", () => {
    render(<VisNetworkCanvas
      model={model}
      focusedNodeId="caller"
      physicsRunning={false}
      layoutStyle="automatic"
      forceLabels={false}
      isolatedNodeIds={new Set(["caller"])}
      isolatedEdgeIds={new Set()}
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

    expect(mock.updates.some((items) => items.some((item) =>
      item.id === "callee" && item.hidden === true))).toBe(true);
    expect(mock.updates.some((items) => items.some((item) =>
      item.id === "caller-callee" && item.hidden === true))).toBe(true);
  });
});
