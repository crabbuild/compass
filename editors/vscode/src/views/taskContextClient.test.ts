import { describe, expect, it } from "vitest";

import { taskContextArguments } from "./taskContextClient";

describe("taskContextClient", () => {
  it("passes literal targets and requests the strict JSON schema", () => {
    expect(taskContextArguments("modify", "Card --literal", "/tmp/graph.json", "/tmp/repo"))
      .toEqual([
        "context", "modify", "Card --literal", "--graph", "/tmp/graph.json",
        "--root", "/tmp/repo", "--format", "json"
      ]);
  });

  it("rejects option injection and empty targets", () => {
    expect(() => taskContextArguments("explain", "--graph", "graph", "repo")).toThrow();
    expect(() => taskContextArguments("explain", "   ", "graph", "repo")).toThrow();
  });
});
