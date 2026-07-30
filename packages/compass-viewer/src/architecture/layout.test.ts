import { describe, expect, it } from "vitest";
import { layoutArchitecture } from "./layout";

const sections = [
  {
    id: "api",
    name: "API",
    nodeCount: 20,
    totalNodeCount: 20,
    internalCallCount: 12,
    incomingCalls: 0,
    outgoingCalls: 30,
    scopes: { production: 20, test: 0, generated: 0, vendor: 0, unknown: 0 }
  },
  {
    id: "storage",
    name: "Storage",
    nodeCount: 10,
    totalNodeCount: 10,
    internalCallCount: 5,
    incomingCalls: 30,
    outgoingCalls: 0,
    scopes: { production: 10, test: 0, generated: 0, vendor: 0, unknown: 0 }
  }
];

const routes = [{
  id: "api→storage",
  sourceSection: "api",
  targetSection: "storage",
  calls: 30,
  extracted: 24,
  inferred: 6,
  ambiguous: 0
}];

describe("layoutArchitecture", () => {
  it("places source-biased subsystems before their targets deterministically", () => {
    const first = layoutArchitecture(sections, routes);
    const second = layoutArchitecture(sections, routes);
    expect(first).toEqual(second);
    expect(first.nodes.find((node) => node.id === "api")!.x)
      .toBeLessThan(first.nodes.find((node) => node.id === "storage")!.x);
    expect(first.routes[0]?.path).toMatch(/^M /);
  });

  it("uses a thicker route for higher call volume", () => {
    const low = layoutArchitecture(sections, [{ ...routes[0]!, calls: 1 }]);
    const high = layoutArchitecture(sections, routes);
    expect(high.routes[0]!.width).toBeGreaterThan(low.routes[0]!.width);
  });

  it("creates aligned flow lanes and rounded orthogonal routes", () => {
    const layout = layoutArchitecture(sections, routes);
    expect(layout.lanes).toHaveLength(2);
    expect(layout.lanes.map((lane) => lane.label)).toEqual(["Entry", "Dependencies"]);
    expect(layout.routes[0]).toMatchObject({ direction: "forward" });
    expect(layout.routes[0]!.path).toContain(" H ");
    expect(layout.routes[0]!.path).toContain(" V ");
    expect(layout.routes[0]!.path).toContain(" Q ");
  });

  it("applies dragged positions and reconnects routes to the moved cards", () => {
    const automatic = layoutArchitecture(sections, routes);
    const moved = layoutArchitecture(
      sections,
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
      ...sections[0]!,
      id: `stage-${index}`,
      name: `Stage ${index}`,
      incomingCalls: index === 0 ? 0 : 10,
      outgoingCalls: index === 11 ? 0 : 10
    }));
    const chainRoutes = Array.from({ length: 11 }, (_, index) => ({
      ...routes[0]!,
      id: `route-${index}`,
      sourceSection: `stage-${index}`,
      targetSection: `stage-${index + 1}`
    }));
    const layout = layoutArchitecture(chainSections, chainRoutes);
    const columns = chainSections.map((section) =>
      layout.nodes.find((node) => node.id === section.id)!.column
    );

    expect(columns).toEqual([...columns].sort((left, right) => left - right));
    expect(new Set(columns).size).toBeGreaterThan(2);
  });

  it("fans routes across separate card ports instead of stacking every line", () => {
    const destinations = Array.from({ length: 4 }, (_, index) => ({
      ...sections[1]!,
      id: `storage-${index}`,
      name: `Storage ${index}`
    }));
    const fanoutRoutes = destinations.map((destination, index) => ({
      ...routes[0]!,
      id: `fanout-${index}`,
      targetSection: destination.id
    }));
    const layout = layoutArchitecture(
      [sections[0]!, ...destinations],
      fanoutRoutes
    );
    const routeStarts = layout.routes.map((route) => route.path.match(/^M [^ ]+ ([^ ]+)/)?.[1]);

    expect(new Set(routeStarts).size).toBe(fanoutRoutes.length);
  });
});
