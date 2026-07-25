import { describe, expect, it } from "vitest";
import { graphReducer, initialGraphState } from "./state";

describe("graphReducer", () => {
  it("pauses on focus and does not resume when focus clears", () => {
    const focused = graphReducer(initialGraphState, {
      type: "focus",
      nodeId: "n1"
    });
    expect(focused.physicsRunning).toBe(false);
    expect(graphReducer(focused, { type: "clearFocus" }).physicsRunning).toBe(false);
  });

  it("only resumes through the explicit physics action", () => {
    const settled = graphReducer(initialGraphState, { type: "stabilized" });
    expect(settled.physicsRunning).toBe(false);
    expect(graphReducer(settled, { type: "stabilized" })).toBe(settled);
    expect(graphReducer(settled, {
      type: "setPhysics",
      running: true
    }).physicsRunning).toBe(true);
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
});
