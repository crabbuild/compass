// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { VisNetworkCanvas, type GraphCanvasHandle } from "./VisNetworkCanvas";

const mock = vi.hoisted(() => ({
  dataSets: [] as Array<Array<Record<string, unknown>>>,
  networks: 0,
  networkOptions: [] as Array<Record<string, unknown>>,
  optionUpdates: [] as Array<Record<string, unknown>>,
  fits: [] as Array<Record<string, unknown> | undefined>,
  moves: [] as Array<Record<string, unknown>>,
  connectedNodeRequests: [] as string[],
  updates: [] as Array<Array<Record<string, unknown>>>,
  movedNodes: [] as Array<{ id: string; x: number; y: number }>,
  simulationStarts: 0,
  simulationStops: 0,
  eventHandlers: new Map<string, Array<() => void>>(),
  positionScale: 10
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
    constructor(_container: unknown, _data: unknown, options: Record<string, unknown>) {
      mock.networks += 1;
      mock.networkOptions.push(options);
    }

    setOptions(options: Record<string, unknown>) { mock.optionUpdates.push(options); }
    stopSimulation() { mock.simulationStops += 1; }
    startSimulation() { mock.simulationStarts += 1; }
    fit(options?: Record<string, unknown>) { mock.fits.push(options); }
    destroy() {}
    on(event: string, callback: () => void) {
      const handlers = mock.eventHandlers.get(event) ?? [];
      handlers.push(callback);
      mock.eventHandlers.set(event, handlers);
    }
    once(event: string, callback: () => void) {
      const wrapped = () => {
        callback();
        const handlers = mock.eventHandlers.get(event) ?? [];
        mock.eventHandlers.set(event, handlers.filter((handler) => handler !== wrapped));
      };
      const handlers = mock.eventHandlers.get(event) ?? [];
      handlers.push(wrapped);
      mock.eventHandlers.set(event, handlers);
    }
    getConnectedNodes(id: string) {
      mock.connectedNodeRequests.push(id);
      return id === "caller" ? ["callee"] : [];
    }
    getViewPosition() { return { x: 0, y: 0 }; }
    getScale() { return 1; }
    getPositions(ids: string[] = ["caller", "callee"]) {
      return Object.fromEntries(ids.map((id, index) => [id, {
        x: index * mock.positionScale,
        y: 0
      }]));
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
    { id: "caller", label: "caller", community: 0, kind: "function" },
    { id: "callee", label: "callee", community: 0, kind: "class" }
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
    mock.networkOptions.length = 0;
    mock.optionUpdates.length = 0;
    mock.fits.length = 0;
    mock.moves.length = 0;
    mock.connectedNodeRequests.length = 0;
    mock.updates.length = 0;
    mock.movedNodes.length = 0;
    mock.simulationStarts = 0;
    mock.simulationStops = 0;
    mock.eventHandlers.clear();
    mock.positionScale = 10;
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

  it("passes the paused physics state into network construction", () => {
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

    expect(mock.networkOptions[0]?.physics).toMatchObject({ enabled: false });
  });

  it("keeps paused physics disabled when solver spacing changes", () => {
    render(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={false}
      layoutStyle="automatic"
      layoutSpacing={1.25}
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

    expect(mock.optionUpdates.some((options) => (
      (options.physics as { enabled?: boolean } | undefined)?.enabled === false
    ))).toBe(true);
  });

  it("uses semantic node shapes and relationship colors in community detail", () => {
    render(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={false}
      layoutStyle="automatic"
      forceLabels={false}
      semanticDetail
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

    expect(mock.dataSets[0]?.map((node) => [node.id, node.shape])).toEqual([
      ["caller", "dot"],
      ["callee", "diamond"]
    ]);
    expect(mock.dataSets[1]?.[0]?.color).toEqual({ color: "#5fa8ff", opacity: 0.35 });
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
    expect(mock.movedNodes[0]?.x).toBeGreaterThanOrEqual(18);
    expect(mock.simulationStarts).toBeGreaterThan(startsBeforeResume);
  });

  it("reports stabilization after every explicit relayout", () => {
    const onStabilized = vi.fn();
    const view = render(<VisNetworkCanvas
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
      onStabilized={onStabilized}
    />);

    for (const handler of [...mock.eventHandlers.get("stabilizationIterationsDone") ?? []]) {
      handler();
    }
    view.rerender(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={true}
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
      onStabilized={onStabilized}
    />);
    for (const handler of [...mock.eventHandlers.get("stabilizationIterationsDone") ?? []]) {
      handler();
    }

    expect(onStabilized).toHaveBeenCalledTimes(2);
  });

  it("stops layout motion before publishing the stabilized canvas", () => {
    const onStabilized = vi.fn();
    render(<VisNetworkCanvas
      model={model}
      focusedNodeId={null}
      physicsRunning={true}
      layoutStyle="automatic"
      forceLabels={false}
      hiddenCommunities={new Set()}
      hiddenChanges={new Set()}
      onFocus={vi.fn()}
      onOpenSource={vi.fn()}
      onOpenRelationshipSource={vi.fn()}
      onInteractionStart={vi.fn()}
      onHover={vi.fn()}
      onHoverEdge={vi.fn()}
      onClear={vi.fn()}
      onStabilized={onStabilized}
    />);

    const stopsBeforeStabilizing = mock.simulationStops;
    for (const handler of [...mock.eventHandlers.get("stabilizationIterationsDone") ?? []]) {
      handler();
    }

    expect(mock.simulationStops).toBeGreaterThan(stopsBeforeStabilizing);
    expect(onStabilized).toHaveBeenCalledTimes(1);
  });

  it("stops an active simulation when a node is hovered", () => {
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

    const stopsBeforeHover = mock.simulationStops;
    const hoverNodeHandlers = mock.eventHandlers.get("hoverNode") ?? [];
    for (const handler of hoverNodeHandlers) {
      (handler as unknown as (parameters: Record<string, unknown>) => void)({
        node: "caller",
        pointer: { DOM: { x: 10, y: 20 } }
      });
    }

    expect(mock.simulationStops).toBeGreaterThan(stopsBeforeHover);
  });

  it("scales the reheat so motion stays visible when a large graph is fit", () => {
    const callbacks = {
      onFocus: vi.fn(),
      onOpenSource: vi.fn(),
      onOpenRelationshipSource: vi.fn(),
      onHover: vi.fn(),
      onHoverEdge: vi.fn(),
      onClear: vi.fn(),
      onStabilized: vi.fn()
    };
    mock.positionScale = 10_000;
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

    expect(mock.movedNodes[0]?.x).toBeCloseTo(96);
    expect(screen.getByRole("region", { name: "Interactive Compass code graph" }))
      .toHaveAttribute("data-physics-running", "true");
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
