// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import {
  GraphMinimap,
  graphMinimapGeometry,
  type GraphMinimapSnapshot
} from "./GraphMinimap";

const snapshot: GraphMinimapSnapshot = {
  positions: new Map([
    ["a", { x: -100, y: -50 }],
    ["b", { x: 100, y: 50 }]
  ]),
  viewport: { left: -50, top: -25, width: 100, height: 50 }
};

const model: GraphViewModel = {
  schema: "compass.viewer.graph/1",
  title: "Minimap fixture",
  stats: { nodes: 2, edges: 1, communities: 1, aggregated: false },
  nodes: [
    { id: "a", label: "A", community: 0 },
    { id: "b", label: "B", community: 0 }
  ],
  edges: [{ id: "a-b", source: "a", target: "b", relation: "calls" }],
  communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
  hyperedges: []
};

describe("GraphMinimap", () => {
  afterEach(cleanup);

  it("projects and unprojects graph coordinates", () => {
    const geometry = graphMinimapGeometry(snapshot);
    const projected = geometry.project({ x: 20, y: 10 });
    expect(geometry.unproject(projected).x).toBeCloseTo(20);
    expect(geometry.unproject(projected).y).toBeCloseTo(10);
    expect(geometry.viewport.width).toBeGreaterThan(0);
  });

  it("renders topology and navigates from a click", () => {
    const onNavigate = vi.fn();
    render(<GraphMinimap
      model={model}
      snapshot={snapshot}
      focusedNodeId="a"
      onNavigate={onNavigate}
    />);
    const minimap = screen.getByRole("button", {
      name: "Graph minimap. Click to reposition the viewport"
    });
    vi.spyOn(minimap, "getBoundingClientRect").mockReturnValue({
      left: 0,
      top: 0,
      width: 176,
      height: 108,
      right: 176,
      bottom: 108,
      x: 0,
      y: 0,
      toJSON: vi.fn()
    });
    fireEvent.click(minimap, { clientX: 88, clientY: 54 });
    expect(onNavigate).toHaveBeenCalledOnce();
    expect(minimap.querySelectorAll("circle")).toHaveLength(2);
    expect(minimap.querySelectorAll("line")).toHaveLength(1);
  });
});
