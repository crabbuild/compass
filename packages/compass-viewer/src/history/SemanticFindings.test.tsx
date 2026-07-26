import { parsePatchFiles } from "@pierre/diffs";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { SemanticFindings } from "./SemanticFindings";
import { minimumDiffHeight, normalizeSourcePatch } from "./SourceChanges";

globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

describe("SemanticFindings", () => {
  it("normalizes GitHub hunk fragments into a complete one-file patch", () => {
    expect(normalizeSourcePatch({
      old_path: "config/files.go",
      new_path: "config/files.go",
      status: "modified",
      patch: "@@ -2,2 +2,2 @@\n-old\n+new\n"
    })).toContain(
      "diff --git a/config/files.go b/config/files.go\n"
      + "--- a/config/files.go\n"
      + "+++ b/config/files.go\n"
      + "@@ -2,2 +2,2 @@"
    );
  });

  it("reserves enough height for every compact diff hunk", () => {
    const patch = [
      "diff --git a/example.ts b/example.ts",
      "--- a/example.ts",
      "+++ b/example.ts",
      "@@ -1,0 +1,2 @@",
      "+const first = true;",
      "+const second = true;",
      "@@ -4 +6 @@",
      "-const value = 1;",
      "+const value = 2;",
      ""
    ].join("\n");
    const fileDiff = parsePatchFiles(patch, "height-test", true)[0]?.files?.[0];

    expect(fileDiff).toBeDefined();
    expect(minimumDiffHeight(fileDiff!, "split")).toBe(137);
    expect(minimumDiffHeight(fileDiff!, "unified")).toBe(156);
  });

  it("renders source-only changes as readable evidence instead of a raw report dump", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
        <SemanticFindings
          report={{
            schema: "compass.semantic_diff.report/1",
            comparison: { old_revision: "parent", new_revision: "current" },
            source_changes: [{
              old_path: "Cargo.toml",
              new_path: "Cargo.toml",
              status: "modified",
              hunks: [{ old_start: 5, old_lines: 1, new_start: 5, new_lines: 1 }],
              patch: "-version = \"3.1.6\"\n+version = \"3.1.7\""
            }],
            findings: [],
            completeness: {
              identity: "complete",
              source_delta: "complete",
              call_resolution: "unavailable",
              test_mapping: "unavailable"
            }
          }}
        />
    ));

    expect(container.querySelector("h2")?.textContent).toBe("Source changes");
    expect(container.textContent).toContain("Cargo.toml");
    expect(container.textContent).toContain("+1−1");
    expect(container.textContent).toContain("No semantic graph findings for this comparison.");
    expect(container.textContent).not.toContain("\"source_changes\"");
    root.unmount();
  });
});
