import { describe, expect, it } from "vitest";
import { buildInitArgs, buildUpdateArgs, buildWatchArgs } from "./buildArguments";

describe("Compass argument builders", () => {
  it("preserves every argument as a separate process value", () => {
    expect(buildInitArgs({
      root: "/repo",
      includes: ["src/**"],
      excludes: ["vendor/**", "$(touch nope)"],
      force: false
    })).toEqual([
      "init", "/repo", "--include", "src/**", "--exclude", "vendor/**",
      "--exclude", "$(touch nope)", "--yes", "--events", "jsonl"
    ]);
  });

  it("builds update and watch commands", () => {
    expect(buildUpdateArgs({ root: "/repo", noViz: true }))
      .toEqual(["update", "/repo", "--no-viz", "--events", "jsonl"]);
    expect(buildWatchArgs({ root: "/repo", debounceSeconds: 0.4, poll: true }))
      .toEqual([
        "watch", "/repo", "--debounce", "0.4", "--poll", "--events", "jsonl"
      ]);
  });
});
