import { describe, expect, it } from "vitest";
import type { GraphViewModel } from "../contracts/graph";
import { STATIC_LAYOUT_NODE_THRESHOLD } from "./renderingProfile";
import { graphReducer, initialGraphState, initialGraphStateForModel } from "./state";

describe("graphReducer", () => {
  it("starts every automatic graph with a static selectable layout", () => {
    const model: GraphViewModel = {
      schema: "compass.viewer.graph/1",
      title: "Interactive fixture",
      stats: { nodes: 2, edges: 1, communities: 1, aggregated: false },
      nodes: [
        { id: "caller", label: "Caller", community: 0 },
        { id: "callee", label: "Callee", community: 0 }
      ],
      edges: [{
        id: "caller-callee",
        source: "caller",
        target: "callee",
        relation: "calls",
        confidence: "extracted"
      }],
      communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
      hyperedges: []
    };

    expect(initialGraphStateForModel(model)).toMatchObject({
      layoutStyle: "automatic",
      physicsRunning: false,
      initialLayoutPending: false
    });
  });

  it("starts large graphs with their deterministic layout visible and paused", () => {
    const model: GraphViewModel = {
      schema: "compass.viewer.graph/1",
      title: "Large fixture",
      stats: {
        nodes: STATIC_LAYOUT_NODE_THRESHOLD,
        edges: 0,
        communities: 1,
        aggregated: false
      },
      nodes: Array.from({ length: STATIC_LAYOUT_NODE_THRESHOLD }, (_, index) => ({
        id: `n-${index}`,
        label: `Node ${index}`,
        community: 0
      })),
      edges: [],
      communities: [{ id: 0, label: "Core", color: "#4e79a7", hidden: false }],
      hyperedges: []
    };

    expect(initialGraphStateForModel(model)).toMatchObject({
      physicsRunning: false,
      initialLayoutPending: false
    });
  });

  it("pauses on focus and does not resume when focus clears", () => {
    const focused = graphReducer(initialGraphState, {
      type: "focus",
      nodeId: "n1"
    });
    expect(focused.physicsRunning).toBe(false);
    expect(graphReducer(focused, { type: "clearFocus" }).physicsRunning).toBe(false);
  });

  it("disables isolation when its focus is cleared", () => {
    const focused = {
      ...initialGraphState,
      focusedNodeId: "n1",
      isolateSelection: true
    };

    expect(graphReducer(focused, { type: "clearFocus" })).toMatchObject({
      focusedNodeId: null,
      isolateSelection: false
    });
  });

  it("only resumes through the explicit physics action", () => {
    const settled = graphReducer(initialGraphState, { type: "stabilized" });
    expect(settled.physicsRunning).toBe(false);
    expect(settled.initialLayoutPending).toBe(false);
    expect(graphReducer(settled, { type: "stabilized" })).toBe(settled);
    const resumed = graphReducer(settled, {
      type: "setPhysics",
      running: true
    });
    expect(resumed.physicsRunning).toBe(true);
    expect(resumed.initialLayoutPending).toBe(false);
  });

  it("reveals the current layout and stops initial physics", () => {
    const revealed = graphReducer(initialGraphState, { type: "revealLayout" });

    expect(revealed.physicsRunning).toBe(false);
    expect(revealed.initialLayoutPending).toBe(false);
  });

  it("uses fixed positioning for a selected layout style", () => {
    const grid = graphReducer(initialGraphState, {
      type: "setLayout",
      layout: "grid",
      runPhysics: false
    });
    expect(grid).toMatchObject({
      layoutStyle: "grid",
      physicsRunning: false,
      initialLayoutPending: false
    });
  });

  it("can hide or reveal every community in one action", () => {
    const hidden = graphReducer(initialGraphState, {
      type: "setHiddenCommunities",
      communityIds: [1, 2]
    });
    expect([...hidden.hiddenCommunities]).toEqual([1, 2]);
    expect(graphReducer(hidden, {
      type: "setHiddenCommunities",
      communityIds: []
    }).hiddenCommunities.size).toBe(0);
  });

  it("toggles relationship labels independently from node labels", () => {
    const withNodeLabels = graphReducer(initialGraphState, {
      type: "setLabels",
      visible: true
    });
    const withBoth = graphReducer(withNodeLabels, {
      type: "setEdgeLabels",
      visible: true
    });

    expect(withBoth.forceLabels).toBe(true);
    expect(withBoth.showEdgeLabels).toBe(true);
    expect(graphReducer(withBoth, {
      type: "setEdgeLabels",
      visible: false
    }).forceLabels).toBe(true);
  });

  it("keeps bounded exploration controls in graph state", () => {
    const isolated = graphReducer(initialGraphState, {
      type: "setIsolation",
      isolated: true
    });
    const deep = graphReducer(isolated, {
      type: "setNeighborhoodDepth",
      depth: 99
    });
    const directed = graphReducer(deep, {
      type: "setEdgeDirection",
      direction: "outgoing"
    });
    const spaced = graphReducer(directed, {
      type: "setLayoutSpacing",
      spacing: 1.5
    });
    const hiddenMap = graphReducer(spaced, {
      type: "setMinimap",
      visible: false
    });

    expect(hiddenMap).toMatchObject({
      isolateSelection: true,
      neighborhoodDepth: 4,
      edgeDirection: "outgoing",
      layoutSpacing: 1.5,
      showMinimap: false
    });
  });
});
