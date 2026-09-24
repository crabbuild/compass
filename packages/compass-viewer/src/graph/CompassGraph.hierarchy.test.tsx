// @vitest-environment jsdom

import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { CommunityHierarchyViewSchema } from "../contracts/hierarchy";
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

function levelModel(ids: string[], edges: Array<[string, string]>) {
  return {
    schema: "compass.viewer.graph/1" as const,
    title: "Fixture",
    stats: { nodes: ids.length, edges: edges.length, communities: ids.length, aggregated: true },
    nodes: ids.map((id) => ({
      id,
      label: id === "0" ? "src/runtime" : `src/runtime/${id}`,
      community: Number(id),
      memberCount: 2,
      size: 40
    })),
    edges: edges.map(([source, target], position) => ({
      id: `edge-${position}`, source, target, relation: "calls", weight: 2
    })),
    communities: [],
    hyperedges: []
  };
}

function baseModel(): GraphViewModel {
  return {
    schema: "compass.viewer.graph/1",
    title: "Fixture",
    stats: { nodes: 4, edges: 1, communities: 2, aggregated: true },
    nodes: [
      { id: "a", label: "alpha()", community: 0, memberCount: 2 },
      { id: "b", label: "beta()", community: 0, memberCount: 2 },
      { id: "c", label: "gamma()", community: 1, memberCount: 2 },
      { id: "d", label: "delta()", community: 1, memberCount: 2 }
    ],
    edges: [{ id: "e", source: "a", target: "c", relation: "calls" }],
    communities: [],
    hyperedges: []
  };
}

function hierarchy() {
  return CommunityHierarchyViewSchema.parse({
    schema: "compass.viewer.hierarchy/1",
    budgetIdentity: "community-hierarchy-budget/v1",
    mergePolicy: "relationship-then-location-affinity/v1",
    rootTarget: 24,
    levelTarget: 300,
    maxLevels: 4,
    budgetSatisfied: true,
    finestCommunityCount: 2,
    finestSignature: `sha256:${"0".repeat(64)}`,
    boundaryKinds: ["route"],
    levels: [
      {
        level: 0,
        merge: "locationAffinity" as const,
        groupCount: 1,
        memberCount: 4,
        groups: [{
          index: 0,
          label: "src/runtime",
          labelRule: "dominantDirectory" as const,
          labelGeneric: false,
          memberCount: 4,
          childIndices: [0, 1],
          cohesion: 0,
          conductance: 0,
          boundaryKinds: { route: 1 },
          detailAvailable: false
        }],
        model: levelModel(["0"], [])
      },
      {
        level: 1,
        merge: "relationship" as const,
        resolution: 1,
        groupCount: 2,
        memberCount: 4,
        groups: [
          {
            index: 0,
            community: 0,
            label: "src/runtime/a",
            labelRule: "dominantDirectory" as const,
            labelGeneric: false,
            memberCount: 2,
            childIndices: [],
            cohesion: 1,
            conductance: 0,
            boundaryKinds: {},
            detailAvailable: false
          },
          {
            index: 1,
            community: 1,
            label: "src/runtime/b",
            labelRule: "dominantDirectory" as const,
            labelGeneric: false,
            memberCount: 2,
            childIndices: [],
            cohesion: 0,
            conductance: 0.5,
            boundaryKinds: {},
            detailAvailable: false
          }
        ],
        model: levelModel(["0", "1"], [["0", "1"]])
      }
    ]
  });
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

function breadcrumb(): string {
  return Array.from(document.querySelectorAll(".compass-graph-breadcrumb"))
    .map((element) => element.textContent ?? "")
    .join(" | ");
}

/**
 * The stage is inert while an automatic layout settles, so these tests drive
 * the controls by their accessible label rather than through role queries.
 */
function control(label: string): HTMLElement {
  const labelled = document.querySelector<HTMLElement>(`[aria-label="${label}"]`);
  if (labelled) return labelled;
  const named = Array.from(document.querySelectorAll("button"))
    .find((button) => (button.textContent ?? "").trim() === label);
  if (!named) {
    throw new Error(`missing control ${label}`);
  }
  return named;
}

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    unobserve() {}
    disconnect() {}
  });
});

afterEach(() => {
  cleanup();
  mock.pendingDataSets.length = 0;
  mock.canvases.length = 0;
  mock.eventHandlers.clear();
  vi.unstubAllGlobals();
});

describe("hierarchy level navigation", () => {
  it("opens on the published root level, not a derived overview", () => {
    render(
      <CompassGraph
        model={baseModel()}
        hierarchy={hierarchy()}
        host={{ openSource: () => undefined, openCommunity: () => undefined }}
      />
    );
    expect(control("Level 0")).toBeDefined();
    expect(control("Level 1")).toBeDefined();
    expect(control("Symbols")).toBeDefined();
    expect(latestNodes().map((node) => node.id)).toEqual(["0"]);
  });

  it("descends into a group's children and back out through the breadcrumb", () => {
    render(
      <CompassGraph
        model={baseModel()}
        hierarchy={hierarchy()}
        host={{ openSource: () => undefined, openCommunity: () => undefined }}
      />
    );
    doubleClickNode("0");
    expect(latestNodes().map((node) => node.id)).toEqual(["0", "1"]);
    expect(breadcrumb()).toContain("Repository");
    expect(breadcrumb()).toContain("src/runtime");

    fireEvent.click(control("Repository"));
    expect(latestNodes().map((node) => node.id)).toEqual(["0"]);
  });

  it("switches to the symbol canvas from any level", () => {
    render(
      <CompassGraph
        model={baseModel()}
        hierarchy={hierarchy()}
        host={{ openSource: () => undefined, openCommunity: () => undefined }}
      />
    );
    doubleClickNode("0");
    fireEvent.click(control("Symbols"));
    expect(latestNodes().map((node) => node.id).sort())
      .toEqual(["a", "b", "c", "d"]);
  });

  it("opens on the level the export asked for", () => {
    render(
      <CompassGraph
        model={baseModel()}
        hierarchy={hierarchy()}
        initialLevel={1}
        host={{ openSource: () => undefined, openCommunity: () => undefined }}
      />
    );
    expect(latestNodes().map((node) => node.id)).toEqual(["0", "1"]);
  });

  it("falls back to the root when the requested level does not exist", () => {
    render(
      <CompassGraph
        model={baseModel()}
        hierarchy={hierarchy()}
        initialLevel={7}
        host={{ openSource: () => undefined, openCommunity: () => undefined }}
      />
    );
    expect(latestNodes().map((node) => node.id)).toEqual(["0"]);
  });

  it("keeps the derived community overview when the export has no hierarchy", () => {
    render(
      <CompassGraph
        model={baseModel()}
        host={{ openSource: () => undefined, openCommunity: () => undefined }}
      />
    );
    expect(document.querySelector('[aria-label="Level 0"]')).toBeNull();
  });
});
