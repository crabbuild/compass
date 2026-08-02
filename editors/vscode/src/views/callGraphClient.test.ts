import { describe, expect, it, vi } from "vitest";
import { runCallGraph } from "./callGraphClient";

describe("callGraphClient", () => {
  it("returns the stable cursor root and forwards bounded graph arguments", async () => {
    const signal = new AbortController().signal;
    const runJson = vi.fn().mockResolvedValue({
      schema: "compass.call_graph/1",
      rootSymbol: "sha256:root",
      direction: "both",
      depth: 1,
      nodes: [],
      edges: [],
      truncated: false,
      continuations: [],
      coverage: {
        resolved: 0,
        inferred: 0,
        ambiguous: 0,
        unresolved: 0,
        evidenceLayer: "structural_graph",
        partial: true,
        limitations: [],
        warning: "Structural graph"
      }
    });
    const session = {
      root: "/repo",
      graphPath: "/repo/compass-out/graph.json",
      processes: { runJson }
    };

    const response = await runCallGraph(
      session as never,
      [
        "--file", "src/lib.rs", "--byte", "120", "--line", "8",
        "--direction", "both", "--depth", "1"
      ],
      signal
    );

    expect(response.rootSymbol).toBe("sha256:root");
    expect(runJson.mock.calls[0]?.[1]).toEqual([
      "call-graph",
      "--file", "src/lib.rs", "--byte", "120", "--line", "8",
      "--direction", "both", "--depth", "1",
      "--max-nodes", "500",
      "--max-edges", "1000",
      "--graph", "/repo/compass-out/graph.json",
      "--format", "json"
    ]);
    expect(runJson.mock.calls[0]?.[3]).toBe(signal);
  });
});
