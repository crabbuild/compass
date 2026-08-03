import { describe, expect, it, vi } from "vitest";
import type { ZodType } from "zod";
import { loadSemanticDiff } from "./diffClient";

function report(schema = "compass.semantic_diff.report/1") {
  return {
    schema,
    comparison: { old_commit: "parent", new_commit: "current", fingerprint: "fingerprint" },
    findings: [],
    feature_groups: [],
    collapsed_groups: [],
    source_changes: [],
    graph_delta: {
      added_nodes: [],
      removed_nodes: [],
      changed_nodes: [],
      added_edges: [],
      removed_edges: [],
      changed_edges: [],
      collapsed_attribute_changes: {}
    },
    entity_display_names: {},
    completeness: {},
    limitations: []
  };
}

describe("semantic-diff client", () => {
  it("requests and validates the current versioned semantic report", async () => {
    const payload = report();
    const runJson = vi.fn(async (
      _root: string,
      _args: readonly string[],
      schema: ZodType
    ) => schema.parse(payload));
    const session = { root: "/repo", processes: { runJson } };

    await expect(loadSemanticDiff(session as never, "parent", "current"))
      .resolves.toMatchObject({ schema: "compass.semantic_diff.report/1" });
    expect(runJson.mock.calls[0]?.[1]).toEqual([
      "diff",
      "parent",
      "current",
      "--format",
      "json"
    ]);
  });

  it("rejects unknown report versions before they reach the webview", async () => {
    const payload = report("compass.semantic_diff.report/2");
    const runJson = vi.fn(async (
      _root: string,
      _args: readonly string[],
      schema: ZodType
    ) => schema.parse(payload));
    const session = { root: "/repo", processes: { runJson } };

    await expect(loadSemanticDiff(session as never, "parent", "current"))
      .rejects.toThrow();
  });
});
