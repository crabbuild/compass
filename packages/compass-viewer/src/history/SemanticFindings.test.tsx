import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { SemanticFindings, SourceChangeEvidence } from "./SemanticFindings";
import { normalizeSourcePatch } from "./SourceChanges";

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

  it("renders source-only changes as readable evidence instead of a raw report dump", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
        <SourceChangeEvidence
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
    expect(container.textContent).not.toContain("\"source_changes\"");
    root.unmount();
  });

  it("renders semantic evidence as comfortable expandable fields", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    flushSync(() => root.render(
      <SemanticFindings
        report={{
          findings: [{
            summary: "Resolver now falls back to the parent organization",
            affected_symbols: ["resolveOrgRef", "pickOrg"],
            confidence: "high"
          }, {
            summary: "Tests cover the new fallback",
            evidence: { file: "resolveref_test.go", line: 42 }
          }]
        }}
      />
    ));

    const cards = container.querySelectorAll<HTMLDetailsElement>(
      ".history-finding-list details"
    );
    expect(cards).toHaveLength(2);
    expect(cards[0]?.open).toBe(true);
    expect(cards[1]?.open).toBe(false);
    expect(container.textContent).toContain("2 evidence fields");
    expect(container.textContent).toContain("Affected symbols");
    expect(container.textContent).toContain("resolveOrgRef");
    expect(container.textContent).not.toContain("\"affected_symbols\"");
    root.unmount();
  });
});
