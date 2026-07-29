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
});
