import { describe, expect, it } from "vitest";
import {
  currentGraphExportArgs,
  historicalGraphExportArgs
} from "./communityArguments";

describe("Compass community export arguments", () => {
  it("uses canonical current JSON export with the immutable graph snapshot", () => {
    expect(currentGraphExportArgs("/tmp/snapshot/graph.json", 8000, 7)).toEqual([
      "export",
      "json",
      "--graph",
      "/tmp/snapshot/graph.json",
      "--node-limit",
      "8000",
      "--community",
      "7"
    ]);
  });

  it("uses the exact historical revision, configured limit, and community", () => {
    expect(historicalGraphExportArgs("abc123", "/tmp/result", 9000, 4)).toEqual([
      "history",
      "export",
      "abc123",
      "--format",
      "json",
      "--node-limit",
      "9000",
      "--output",
      "/tmp/result",
      "--community",
      "4"
    ]);
  });
});
