import { describe, expect, it } from "vitest";
import { layoutArchitecture } from "./layout";

const groups = [
  {
    id: "api",
    name: "API",
    nodeCount: 20,
    totalNodeCount: 20,
    internalRelationshipCount: 12,
    incomingRelationships: 0,
    outgoingRelationships: 30,
    scopes: { production: 20, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0 }
  },
  {
    id: "storage",
    name: "Storage",
    nodeCount: 10,
    totalNodeCount: 10,
    internalRelationshipCount: 5,
    incomingRelationships: 30,
    outgoingRelationships: 0,
    scopes: { production: 10, test: 0, generated: 0, vendor: 0, documentation: 0, unknown: 0 }
  }
];

const routes = [{
  id: "api→storage",
  sourceGroup: "api",
  targetGroup: "storage",
  relationships: 30,
  extracted: 24,
  inferred: 6,
  ambiguous: 0
}];

describe("layoutArchitecture", () => {
  it("places source-biased subsystems before their targets deterministically", () => {
    const first = layoutArchitecture(groups, routes);
    const second = layoutArchitecture(groups, routes);
    expect(first).toEqual(second);
    expect(first.nodes.find((node) => node.id === "api")!.x)
      .toBeLessThan(first.nodes.find((node) => node.id === "storage")!.x);
    expect(first.routes[0]?.path).toMatch(/^M /);
  });

  it("uses a thicker route for higher call volume", () => {
    const low = layoutArchitecture(groups, [{ ...routes[0]!, relationships: 1 }]);
    const high = layoutArchitecture(groups, routes);
    expect(high.routes[0]!.width).toBeGreaterThan(low.routes[0]!.width);
  });

  it("creates dependency-depth lanes and separated curved routes", () => {
    const layout = layoutArchitecture(groups, routes);
    expect(layout.lanes).toHaveLength(2);
    expect(layout.lanes.map((lane) => lane.label)).toEqual(["Callers", "Dependencies"]);
    expect(layout.routes[0]).toMatchObject({ direction: "forward" });
    expect(layout.routes[0]!.path).toContain(" C ");
    expect(layout.routes[0]!.path).not.toContain(" V ");
  });

  it("labels the selected subsystem and its immediate call direction explicitly", () => {
    const layout = layoutArchitecture(groups, routes, undefined, {}, "api");
    expect(layout.lanes.map((lane) => lane.label)).toEqual([
      "Focus",
      "Direct dependencies"
    ]);
  });

  it("applies dragged positions and reconnects routes to the moved cards", () => {
    const automatic = layoutArchitecture(groups, routes);
    const moved = layoutArchitecture(
      groups,
      routes,
      undefined,
      { api: { x: 420, y: 510 } }
    );
    expect(moved.nodes.find((node) => node.id === "api")).toMatchObject({
      x: 420,
      y: 510
    });
    expect(moved.routes[0]!.path).not.toBe(automatic.routes[0]!.path);
  });

  it("keeps a long directed chain in reading order across balanced stages", () => {
    const chainSections = Array.from({ length: 12 }, (_, index) => ({
      ...groups[0]!,
      id: `stage-${index}`,
      name: `Stage ${index}`,
      incomingRelationships: index === 0 ? 0 : 10,
      outgoingRelationships: index === 11 ? 0 : 10
    }));
    const chainRoutes = Array.from({ length: 11 }, (_, index) => ({
      ...routes[0]!,
      id: `route-${index}`,
      sourceGroup: `stage-${index}`,
      targetGroup: `stage-${index + 1}`
    }));
    const layout = layoutArchitecture(chainSections, chainRoutes);
    const columns = chainSections.map((section) =>
      layout.nodes.find((node) => node.id === section.id)!.column
    );

    expect(columns).toEqual([...columns].sort((left, right) => left - right));
    expect(new Set(columns).size).toBeGreaterThan(2);
    expect(new Set(columns).size).toBeLessThanOrEqual(12);
  });

  it("keeps the dominant cycle direction forward and moves one edge to a feedback rail", () => {
    const cycleSections = ["input", "core", "output"].map((id, index) => ({
      ...groups[0]!,
      id,
      name: id,
      incomingRelationships: index === 0 ? 2 : 12,
      outgoingRelationships: index === 2 ? 2 : 12
    }));
    const cycleRoutes = [
      { ...routes[0]!, id: "input-core", sourceGroup: "input", targetGroup: "core", relationships: 30 },
      { ...routes[0]!, id: "core-output", sourceGroup: "core", targetGroup: "output", relationships: 28 },
      { ...routes[0]!, id: "output-input", sourceGroup: "output", targetGroup: "input", relationships: 2 }
    ];
    const layout = layoutArchitecture(cycleSections, cycleRoutes);

    expect(layout.routes.filter((route) => route.direction === "forward")).toHaveLength(2);
    expect(layout.routes.find((route) => route.id === "output-input")).toMatchObject({
      direction: "backward"
    });
    expect(layout.routes.find((route) => route.id === "output-input")!.path).toContain(" V ");
  });

  it("uses dependency depth rather than evenly partitioning a fan-out graph", () => {
    const fanoutSections = [
      { ...groups[0]!, id: "entry", name: "Entry" },
      ...Array.from({ length: 5 }, (_, index) => ({
        ...groups[1]!,
        id: `dependency-${index}`,
        name: `Dependency ${index}`
      }))
    ];
    const fanoutRoutes = fanoutSections.slice(1).map((target, index) => ({
      ...routes[0]!,
      id: `entry-${index}`,
      sourceGroup: "entry",
      targetGroup: target.id
    }));
    const layout = layoutArchitecture(fanoutSections, fanoutRoutes);

    expect(layout.lanes).toHaveLength(2);
    expect(layout.nodes.find((node) => node.id === "entry")!.column).toBe(0);
    expect(layout.nodes.filter((node) => node.id !== "entry").every((node) => node.column === 1))
      .toBe(true);
  });

  it("gives a large architecture enough horizontal room to remain readable", () => {
    const denseSections = Array.from({ length: 26 }, (_, index) => ({
      ...groups[0]!,
      id: `section-${index}`,
      name: `Section ${index}`
    }));
    const layout = layoutArchitecture(denseSections, []);

    expect(layout.lanes).toHaveLength(6);
    expect(layout.width).toBeGreaterThan(1800);
    expect(layout.lanes.every((lane) => lane.width >= 288)).toBe(true);
  });

  it("fans routes across separate card ports instead of stacking every line", () => {
    const destinations = Array.from({ length: 4 }, (_, index) => ({
      ...groups[1]!,
      id: `storage-${index}`,
      name: `Storage ${index}`
    }));
    const fanoutRoutes = destinations.map((destination, index) => ({
      ...routes[0]!,
      id: `fanout-${index}`,
      targetGroup: destination.id
    }));
    const layout = layoutArchitecture(
      [groups[0]!, ...destinations],
      fanoutRoutes
    );
    const routeStarts = layout.routes.map((route) => route.path.match(/^M [^ ]+ ([^ ]+)/)?.[1]);

    expect(new Set(routeStarts).size).toBe(fanoutRoutes.length);
  });
});
