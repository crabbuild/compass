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
    expect(graphReducer(settled, {
      type: "setPhysics",
      running: true
    }).physicsRunning).toBe(true);
  });
});
